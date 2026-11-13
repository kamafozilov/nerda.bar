//! Token statistics from Claude Code's own session logs: every assistant message under
//! `<config>/projects/**/*.jsonl` of the last 30 days, counted once, by local hour and model.
//!
//! Claude Code writes a message once per content block and copies earlier messages into a
//! resumed session's file, so the same message id shows up many times. Each id counts once,
//! with the usage of its most complete copy. Files are read from where the last scan stopped.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fs::File;
use std::hash::{BuildHasher, RandomState};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use memchr::memmem;
use serde::Deserialize;

use crate::model;
use crate::prices::Prices;
use crate::time::{self, MONTHS, WEEKDAYS};

/// Days of history kept, today included.
pub const DAYS: i32 = 30;
/// Token kinds, in the order of a [`Tokens`] array.
pub const INPUT: usize = 0;
pub const OUTPUT: usize = 1;
pub const CACHE_WRITE_5M: usize = 2;
pub const CACHE_WRITE_1H: usize = 3;
pub const CACHE_READ: usize = 4;
pub type Tokens = [u64; 5];

/// Days of hourly activity handed to the limit chart: a week's window and the day it started.
pub const ACTIVITY_DAYS: i32 = 8;

/// Request sizes are kept in steps of this many prompt tokens, which is what long-context
/// price tiers are cut at.
pub const CONTEXT_STEP: u64 = 50_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct CellKey {
    /// Local calendar day, as [`time::Local::day`].
    day: i32,
    hour: u8,
    model: u16,
    /// The request's prompt size in [`CONTEXT_STEP`]s.
    context: u8,
}

/// One message as counted: which cell it went to and what it added there.
struct Counted {
    cell: u32,
    tokens: [u32; 5],
}

#[derive(Deserialize)]
struct Line<'a> {
    #[serde(rename = "type", borrow)]
    kind: Cow<'a, str>,
    #[serde(borrow)]
    timestamp: Option<Cow<'a, str>>,
    #[serde(borrow)]
    message: Option<Message<'a>>,
}

#[derive(Deserialize)]
struct Message<'a> {
    #[serde(borrow)]
    id: Option<Cow<'a, str>>,
    #[serde(borrow)]
    model: Option<Cow<'a, str>>,
    usage: Option<Usage>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Usage {
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_input_tokens: u64,
    cache_read_input_tokens: u64,
    cache_creation: Option<CacheCreation>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct CacheCreation {
    ephemeral_5m_input_tokens: u64,
    ephemeral_1h_input_tokens: u64,
}

impl Usage {
    fn tokens(&self) -> [u32; 5] {
        let (write_5m, write_1h) = match &self.cache_creation {
            Some(split) if split.ephemeral_5m_input_tokens + split.ephemeral_1h_input_tokens > 0 => (split.ephemeral_5m_input_tokens, split.ephemeral_1h_input_tokens),
            _ => (self.cache_creation_input_tokens, 0),
        };
        [self.input_tokens, self.output_tokens, write_5m, write_1h, self.cache_read_input_tokens].map(|count| count.min(u32::MAX as u64) as u32)
    }
}

pub struct Scanner {
    /// The `projects` folders to read.
    roots: Vec<PathBuf>,
    /// How far into each file the last scan read.
    cursors: HashMap<PathBuf, u64>,
    /// Counted messages by the hash of their id.
    counted: HashMap<u64, Counted>,
    hasher: RandomState,
    cells: Vec<(CellKey, Tokens)>,
    cell_index: HashMap<CellKey, u32>,
    models: Vec<String>,
    /// Local day and hour by quarter hour of Unix time, so the C library is asked once each.
    local_memo: HashMap<i64, (i32, u8)>,
    buffer: Vec<u8>,
}

impl Scanner {
    pub fn new(config_dirs: &[PathBuf]) -> Scanner {
        Scanner {
            roots: config_dirs.iter().map(|dir| dir.join("projects")).collect(),
            cursors: HashMap::new(),
            counted: HashMap::new(),
            hasher: RandomState::new(),
            cells: Vec::new(),
            cell_index: HashMap::new(),
            models: Vec::new(),
            local_memo: HashMap::new(),
            buffer: Vec::new(),
        }
    }

    /// Reads what the logs gained since the last scan. Returns whether any count changed.
    pub fn scan(&mut self, now: i64) -> bool {
        let today = time::local(now).day;
        let cutoff = now - (DAYS as i64 + 1) * 86_400;
        let mut files = Vec::new();
        for root in self.roots.clone() {
            collect_logs(&root, cutoff, 0, &mut files);
        }
        let mut changed = self.forget_before(today - DAYS);
        for (path, size) in files {
            let from = self.cursors.get(&path).copied().unwrap_or(0);
            // A file that shrank was rewritten: read it again from the start. Message ids make
            // that safe.
            let from = if size < from { 0 } else { from };
            if size == from {
                continue;
            }
            let read = self.read_from(&path, from, cutoff);
            changed |= read.is_some_and(|counted| counted);
        }
        // Everything read was returned to the system or is small; don't hold a big buffer.
        self.buffer = Vec::new();
        changed
    }

    /// Counts the complete lines of `path` from byte `from` on and moves its cursor past them.
    fn read_from(&mut self, path: &Path, from: u64, cutoff: i64) -> Option<bool> {
        let mut file = File::open(path).ok()?;
        file.seek(SeekFrom::Start(from)).ok()?;
        let mut buffer = std::mem::take(&mut self.buffer);
        buffer.clear();
        file.read_to_end(&mut buffer).ok()?;
        // A last line still being written waits for the next scan.
        let complete = memchr::memrchr(b'\n', &buffer).map_or(0, |at| at + 1);
        let mut changed = false;
        let assistant = memmem::Finder::new(br#""type":"assistant""#);
        for line in buffer[..complete].split(|b| *b == b'\n') {
            if assistant.find(line).is_some() {
                changed |= self.count_line(line, cutoff);
            }
        }
        self.cursors.insert(path.to_owned(), from + complete as u64);
        self.buffer = buffer;
        Some(changed)
    }

    fn count_line(&mut self, line: &[u8], cutoff: i64) -> bool {
        let Ok(line) = serde_json::from_slice::<Line>(line) else { return false };
        let (Some(message), Some(timestamp)) = (line.message, line.timestamp) else { return false };
        let (Some(usage), Some(model)) = (message.usage, message.model) else { return false };
        if line.kind != "assistant" || model.starts_with('<') {
            return false;
        }
        let Some(at) = time::parse_iso(&timestamp).filter(|at| *at >= cutoff) else { return false };
        let tokens = usage.tokens();
        if tokens.iter().all(|count| *count == 0) {
            return false;
        }

        let quarter = at.div_euclid(900);
        let (day, hour) = *self.local_memo.entry(quarter).or_insert_with(|| {
            let local = time::local(at);
            (local.day, local.hour)
        });
        let prompt = tokens[INPUT] as u64 + tokens[CACHE_WRITE_5M] as u64 + tokens[CACHE_WRITE_1H] as u64 + tokens[CACHE_READ] as u64;
        let key = CellKey { day, hour, model: self.model_index(&model), context: (prompt / CONTEXT_STEP).min(u8::MAX as u64) as u8 };
        let cell = self.cell(key);

        let Some(id) = message.id.filter(|id| !id.is_empty()) else {
            self.add(cell, tokens);
            return true;
        };
        let id = self.hasher.hash_one(id.as_bytes());
        if let Some((earlier_cell, earlier)) = self.counted.get(&id).map(|counted| (counted.cell, counted.tokens)) {
            if tokens[OUTPUT] <= earlier[OUTPUT] {
                return false;
            }
            // The earlier copy was written before the answer finished: this one replaces it.
            self.subtract(earlier_cell, earlier);
        }
        self.add(cell, tokens);
        self.counted.insert(id, Counted { cell, tokens });
        true
    }

    fn model_index(&mut self, model: &str) -> u16 {
        if let Some(at) = self.models.iter().position(|m| m == model) {
            return at as u16;
        }
        self.models.push(model.to_owned());
        (self.models.len() - 1) as u16
    }

    fn cell(&mut self, key: CellKey) -> u32 {
        *self.cell_index.entry(key).or_insert_with(|| {
            self.cells.push((key, [0; 5]));
            (self.cells.len() - 1) as u32
        })
    }

    fn add(&mut self, cell: u32, tokens: [u32; 5]) {
        for (total, count) in self.cells[cell as usize].1.iter_mut().zip(tokens) {
            *total += count as u64;
        }
    }

    fn subtract(&mut self, cell: u32, tokens: [u32; 5]) {
        for (total, count) in self.cells[cell as usize].1.iter_mut().zip(tokens) {
            *total = total.saturating_sub(count as u64);
        }
    }

    /// Drops what was counted before `first_day`, so a long-running app keeps a month.
    fn forget_before(&mut self, first_day: i32) -> bool {
        if self.cells.iter().all(|(key, _)| key.day >= first_day) {
            return false;
        }
        let mut moved = vec![u32::MAX; self.cells.len()];
        let mut kept = Vec::new();
        for (at, cell) in self.cells.iter().enumerate() {
            if cell.0.day >= first_day {
                moved[at] = kept.len() as u32;
                kept.push(*cell);
            }
        }
        self.cells = kept;
        self.cell_index = self.cells.iter().enumerate().map(|(at, (key, _))| (*key, at as u32)).collect();
        self.counted.retain(|_, counted| {
            counted.cell = moved[counted.cell as usize];
            counted.cell != u32::MAX
        });
        self.local_memo.clear();
        true
    }

    /// The counts as the panel shows them: today, the last 7 days and the last 30, and the
    /// activity of the last week by the hour.
    pub fn view(&self, prices: &Prices, now: i64) -> StatsView {
        let today = time::local(now).day;
        StatsView { ranges: [self.range(prices, today, 1), self.range(prices, today, 7), self.range(prices, today, DAYS)], activity: self.activity(prices, today) }
    }

    /// Each hour's messages by model, weighed by what they would have cost.
    fn activity(&self, prices: &Prices, today: i32) -> Vec<Activity> {
        let mut by_hour: HashMap<(i32, u8, u16), f64> = HashMap::new();
        for (key, tokens) in &self.cells {
            if key.day < today - ACTIVITY_DAYS || key.day > today {
                continue;
            }
            let model = &self.models[key.model as usize];
            let weight = prices.cost(model, key.context as u64 * CONTEXT_STEP, tokens).unwrap_or_else(|| nominal_cost(tokens));
            *by_hour.entry((key.day, key.hour, key.model)).or_default() += weight;
        }
        let mut activity: Vec<Activity> = by_hour
            .into_iter()
            .filter(|(_, weight)| *weight > 0.0)
            .map(|((day, hour, model), weight)| Activity { at: time::local_hour_start(day, hour), model: display_name(&self.models[model as usize]).to_owned(), weight })
            .collect();
        activity.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.model.cmp(&b.model)));
        activity
    }

    fn range(&self, prices: &Prices, today: i32, days: i32) -> RangeView {
        let first = today - days + 1;
        let hourly = days == 1;
        let mut bars: Vec<Bar> = (0..if hourly { 24 } else { days })
            .map(|at| {
                let label = if hourly {
                    format!("{at:02}:00")
                } else {
                    let date = time::local_day(first + at);
                    format!("{} {} {}", WEEKDAYS[date.weekday as usize], date.day_of_month, MONTHS[date.month as usize - 1])
                };
                Bar { label, tokens: 0, cost: 0.0 }
            })
            .collect();
        let mut view = RangeView { tokens: [0; 5], cost: 0.0, unpriced: false, bars: Vec::new(), axis: Vec::new(), axis_per_bar: false, models: Vec::new() };
        let mut by_model: HashMap<String, ModelRow> = HashMap::new();
        for (key, tokens) in &self.cells {
            let total: u64 = tokens.iter().sum();
            if key.day < first || key.day > today || total == 0 {
                continue;
            }
            let model = &self.models[key.model as usize];
            let cost = prices.cost(model, key.context as u64 * CONTEXT_STEP, tokens);
            for (sum, count) in view.tokens.iter_mut().zip(tokens) {
                *sum += count;
            }
            view.cost += cost.unwrap_or(0.0);
            view.unpriced |= cost.is_none();
            let bar = &mut bars[if hourly { key.hour as usize } else { (key.day - first) as usize }];
            bar.tokens += total;
            bar.cost += cost.unwrap_or(0.0);
            let name = display_name(model);
            let row = by_model.entry(name.to_owned()).or_insert_with(|| ModelRow { name: name.to_owned(), tokens: 0, cost: Some(0.0) });
            row.tokens += total;
            row.cost = row.cost.zip(cost).map(|(sum, cost)| sum + cost);
        }
        view.models = by_model.into_values().collect();
        view.models.sort_by(|a, b| b.cost.unwrap_or(0.0).total_cmp(&a.cost.unwrap_or(0.0)).then(b.tokens.cmp(&a.tokens)).then_with(|| a.name.cmp(&b.name)));
        if hourly {
            view.axis = ["00:00", "06:00", "12:00", "18:00", "24:00"].map(str::to_owned).to_vec();
        } else if days <= 7 {
            view.axis = (0..days).map(|at| WEEKDAYS[time::local_day(first + at).weekday as usize].to_owned()).collect();
            view.axis_per_bar = true;
        } else {
            let date = |day: i32| {
                let date = time::local_day(day);
                format!("{} {}", date.day_of_month, MONTHS[date.month as usize - 1])
            };
            view.axis = vec![date(first), date(first + days / 2), date(today)];
        }
        view.bars = bars;
        view
    }
}

/// What tokens of a model without a known price weigh: a mid-range model's prices, so its
/// output still outweighs its cache reads as it would on any price list.
fn nominal_cost(tokens: &Tokens) -> f64 {
    let per_million = [3.0, 15.0, 3.75, 6.0, 0.3];
    tokens.iter().zip(per_million).map(|(count, price)| *count as f64 * price).sum::<f64>() / 1e6
}

/// A model id without its release date: "claude-opus-4-5-20251101" is "claude-opus-4-5".
pub fn display_name(model: &str) -> &str {
    match model.rsplit_once('-') {
        Some((name, date)) if date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit()) => name,
        _ => model,
    }
}

/// The `.jsonl` files under `dir` changed since `cutoff`, with their sizes.
fn collect_logs(dir: &Path, cutoff: i64, depth: usize, out: &mut Vec<(PathBuf, u64)>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else { continue };
        let path = entry.path();
        if kind.is_dir() {
            // projects/<project>/<session>/subagents/<agent>.jsonl is the deepest Claude Code goes.
            if depth < 6 {
                collect_logs(&path, cutoff, depth + 1, out);
            }
        } else if kind.is_file() && path.extension().is_some_and(|ext| ext == "jsonl") {
            let Ok(meta) = entry.metadata() else { continue };
            let modified = meta.modified().ok().and_then(|at| at.duration_since(UNIX_EPOCH).ok()).map_or(i64::MAX, |d| d.as_secs() as i64);
            if modified >= cutoff {
                out.push((path, meta.len()));
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Bar {
    /// "14:00", or "Thu 12 Nov".
    pub label: String,
    pub tokens: u64,
    pub cost: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModelRow {
    pub name: String,
    pub tokens: u64,
    /// `None` when no price is known for the model.
    pub cost: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RangeView {
    pub tokens: Tokens,
    /// What the tokens would cost at API prices, over the models a price is known for.
    pub cost: f64,
    /// Some of the tokens are from models without a known price, so `cost` is a floor.
    pub unpriced: bool,
    pub bars: Vec<Bar>,
    /// Labels under the chart: one per bar, or a few spread across it.
    pub axis: Vec<String>,
    pub axis_per_bar: bool,
    /// Most expensive first.
    pub models: Vec<ModelRow>,
}

impl RangeView {
    pub fn total(&self) -> u64 {
        self.tokens.iter().sum()
    }

    pub fn cost_label(&self) -> String {
        match (self.cost > 0.0, self.unpriced) {
            (true, true) => format!("{}+", model::money(self.cost)),
            (false, true) => "—".to_owned(),
            _ => model::money(self.cost),
        }
    }
}

/// One model's messages in one hour.
#[derive(Clone, Debug, PartialEq)]
pub struct Activity {
    /// Unix seconds of the hour's start.
    pub at: i64,
    /// As [`display_name`].
    pub model: String,
    /// What the messages would have cost at API prices.
    pub weight: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StatsView {
    /// Today, 7 days, 30 days.
    pub ranges: [RangeView; 3],
    /// The last [`ACTIVITY_DAYS`] by the hour, oldest first.
    pub activity: Vec<Activity>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prices::Rate;

    fn line(id: &str, model: &str, at: &str, input: u64, output: u64, write: u64, read: u64) -> String {
        format!(
            r#"{{"parentUuid":"p","isSidechain":false,"message":{{"model":"{model}","id":"{id}","content":[{{"type":"text","text":"a \"quoted\" reply"}}],"usage":{{"input_tokens":{input},"cache_creation_input_tokens":{write},"cache_read_input_tokens":{read},"output_tokens":{output},"cache_creation":{{"ephemeral_1h_input_tokens":{write},"ephemeral_5m_input_tokens":0}}}}}},"requestId":"r","type":"assistant","timestamp":"{at}"}}"#
        )
    }

    fn scanner_over(name: &str, lines: &[String]) -> (Scanner, PathBuf) {
        let dir = std::env::temp_dir().join(format!("nerda-bar-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("projects/demo")).unwrap();
        std::fs::write(dir.join("projects/demo/session.jsonl"), lines.join("\n") + "\n").unwrap();
        (Scanner::new(std::slice::from_ref(&dir)), dir)
    }

    /// Noon UTC on 12 Nov 2026 and the log timestamps around it.
    const NOW: i64 = 1_794_484_800;

    fn prices() -> Prices {
        let mut prices = Prices::default();
        prices.models.insert("claude-opus-4-5".to_owned(), Rate { input: 5.0, output: 25.0, cache_read: Some(0.5), cache_write: Some(6.25), tiers: Vec::new() });
        prices
    }

    #[test]
    fn a_message_counts_once_with_its_fullest_copy() {
        let lines = [
            line("msg_1", "claude-opus-4-5-20251101", "2026-11-12T10:00:00.000Z", 10, 5, 100, 1000),
            line("msg_1", "claude-opus-4-5-20251101", "2026-11-12T10:00:01.000Z", 10, 40, 100, 1000),
            line("msg_1", "claude-opus-4-5-20251101", "2026-11-12T10:00:02.000Z", 10, 40, 100, 1000),
            line("msg_2", "claude-opus-4-5-20251101", "2026-11-12T10:05:00.000Z", 1, 2, 0, 3),
            line("msg_3", "<synthetic>", "2026-11-12T10:06:00.000Z", 1, 2, 0, 3),
            r#"{"type":"user","message":{"content":"mentions \"type\":\"assistant\" in passing"},"timestamp":"2026-11-12T10:06:00.000Z"}"#.to_owned(),
        ];
        let (mut scanner, dir) = scanner_over("dedup", &lines);
        assert!(scanner.scan(NOW));
        let view = scanner.view(&prices(), NOW);
        let month = &view.ranges[2];
        assert_eq!(month.tokens, [11, 42, 0, 100, 1003]);
        assert_eq!(month.models.len(), 1);
        assert_eq!(month.models[0].name, "claude-opus-4-5");
        // 11 in, 42 out, 100 written for an hour (twice the input price), 1003 read.
        let expected = (11.0 * 5.0 + 42.0 * 25.0 + 100.0 * 10.0 + 1003.0 * 0.5) / 1e6;
        assert!((month.cost - expected).abs() < 1e-12, "{} vs {expected}", month.cost);
        assert!(!month.unpriced);
        // Nothing new: a second scan changes nothing.
        assert!(!scanner.scan(NOW));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn only_appended_lines_are_read_and_a_partial_line_waits() {
        let (mut scanner, dir) = scanner_over("append", &[line("msg_1", "claude-opus-4-5", "2026-11-12T10:00:00Z", 10, 5, 0, 0)]);
        scanner.scan(NOW);
        let path = dir.join("projects/demo/session.jsonl");
        let more = line("msg_2", "claude-opus-4-5", "2026-11-12T11:00:00Z", 1, 1, 0, 0);
        let (head, tail) = more.split_at(40);
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str(head);
        std::fs::write(&path, &text).unwrap();
        assert!(!scanner.scan(NOW), "half a line is not counted");
        text.push_str(tail);
        text.push('\n');
        std::fs::write(&path, &text).unwrap();
        assert!(scanner.scan(NOW));
        assert_eq!(scanner.view(&prices(), NOW).ranges[2].total(), 17);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn ranges_keep_their_days_and_unknown_models_have_no_price() {
        let lines = [
            line("msg_1", "claude-opus-4-5", "2026-11-12T10:00:00Z", 100, 0, 0, 0),
            line("msg_2", "claude-opus-4-5", "2026-11-08T10:00:00Z", 20, 0, 0, 0),
            line("msg_3", "mystery-model", "2026-10-20T10:00:00Z", 3, 0, 0, 0),
            line("msg_4", "claude-opus-4-5", "2026-09-01T10:00:00Z", 999, 0, 0, 0),
        ];
        let (mut scanner, dir) = scanner_over("ranges", &lines);
        scanner.scan(NOW);
        let view = scanner.view(&prices(), NOW);
        assert_eq!(view.ranges.each_ref().map(RangeView::total), [100, 120, 123]);
        assert_eq!(view.ranges[0].bars.len(), 24);
        assert_eq!(view.ranges[1].bars.len(), 7);
        assert_eq!(view.ranges[1].bars[6].tokens, 100);
        assert_eq!(view.ranges[1].bars[2].tokens, 20);
        assert_eq!(view.ranges[1].axis.len(), 7);
        assert!(view.ranges[2].unpriced && !view.ranges[1].unpriced);
        assert_eq!(view.ranges[2].models.last().unwrap().cost, None);
        assert!(view.ranges[2].cost_label().ends_with('+'));
        // The week's activity by the hour: November's two messages, not October's.
        assert_eq!(view.activity.len(), 2);
        assert!(view.activity[0].at < view.activity[1].at);
        // 10:00 UTC on the 12th, in whatever zone the test runs in.
        let at = view.activity[1].at;
        assert!(at <= 1_794_477_600 && 1_794_477_600 - at < 3600);
        assert!((view.activity[1].weight - 100.0 * 5.0 / 1e6).abs() < 1e-12);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn old_days_are_forgotten() {
        let (mut scanner, dir) = scanner_over("forget", &[
            line("msg_1", "claude-opus-4-5", "2026-11-12T10:00:00Z", 100, 0, 0, 0),
            line("msg_2", "claude-opus-4-5", "2026-10-20T10:00:00Z", 7, 0, 0, 0),
        ]);
        scanner.scan(NOW);
        assert_eq!(scanner.counted.len(), 2);
        // Twenty days on, the October message is out of the month.
        assert!(scanner.scan(NOW + 20 * 86_400));
        assert_eq!(scanner.counted.len(), 1);
        assert_eq!(scanner.view(&prices(), NOW + 20 * 86_400).ranges[2].total(), 100);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn release_dates_come_off_model_names() {
        assert_eq!(display_name("claude-opus-4-5-20251101"), "claude-opus-4-5");
        assert_eq!(display_name("claude-fable-5-2"), "claude-fable-5-2");
    }
}
