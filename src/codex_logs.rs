//! Token statistics from the Codex CLI's session logs: every response of the last 30 days under
//! `<codex home>/sessions/YYYY/MM/DD/rollout-*.jsonl`, and under `archived_sessions`, where
//! Codex moves a session that was archived, counted by local hour, model and project folder,
//! in the same shape as Claude Code's (see `stats`). Codex compresses older sessions to
//! `.jsonl.zst`; those are read too.
//!
//! A session's `token_count` events carry its running total, and the same total is written
//! again whenever new rate limits arrive, so each response counts as the rise of the total
//! since the event before. The model and the working directory are the ones the latest
//! `turn_context` line names. Files are read from where the last scan stopped, and a session
//! is known by its file's name, so one that moves or is compressed is not counted again.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde_json::Value;

use crate::prices::Prices;
use crate::stats::{CACHE_READ, CACHE_WRITE_5M, DAYS, INPUT, OUTPUT, Scanner, StatsView};
use crate::time;

/// Bytes of a session read at a time.
const CHUNK: u64 = 4 << 20;

/// Where one session's file was read to, and what it had said by then.
#[derive(Default)]
struct Cursor {
    /// Bytes of the session read, as it is before compression.
    offset: u64,
    /// The compressed file's size when it was read: unchanged, there is nothing new in it.
    compressed: Option<u64>,
    model: String,
    /// The session's working directory.
    cwd: String,
    /// The session's running total: input, cached input, cache writes, output.
    total: [u64; 4],
}

pub struct CodexLogs {
    roots: [PathBuf; 2],
    /// By the session's file name, without `.zst`.
    cursors: HashMap<String, Cursor>,
    counts: Scanner,
}

impl CodexLogs {
    pub fn new(home: &Path) -> CodexLogs {
        CodexLogs { roots: [home.join("sessions"), home.join("archived_sessions")], cursors: HashMap::new(), counts: Scanner::new(&[]) }
    }

    /// Reads what the logs gained since the last scan. Returns whether any count changed.
    pub fn scan(&mut self, now: i64) -> bool {
        let cutoff = now - (DAYS as i64 + 1) * 86_400;
        let mut files = Vec::new();
        for root in &self.roots {
            collect(root, cutoff, 0, &mut files);
        }
        // Forgets what fell out of the month.
        let mut changed = self.counts.scan(now);
        for (path, size) in files {
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else { continue };
            let zst = name.ends_with(".zst");
            let session = name.trim_end_matches(".zst").to_owned();
            let mut cursor = self.cursors.remove(&session).unwrap_or_default();
            if zst {
                if cursor.compressed != Some(size) {
                    changed |= self.read(&path, &mut cursor, cutoff, true);
                    cursor.compressed = Some(size);
                }
            } else {
                // A file that shrank was rewritten: read it again from the start.
                if size < cursor.offset {
                    cursor = Cursor::default();
                }
                if size > cursor.offset {
                    changed |= self.read(&path, &mut cursor, cutoff, false);
                }
            }
            self.cursors.insert(session, cursor);
        }
        changed
    }

    /// Counts the complete lines of `path` past the cursor, a chunk at a time, and moves the
    /// cursor past them. A compressed file is read from its start, and what the cursor has
    /// already counted of the session is passed over.
    fn read(&mut self, path: &Path, cursor: &mut Cursor, cutoff: i64, zst: bool) -> bool {
        let Ok(mut file) = File::open(path) else { return false };
        let mut skip = 0;
        let mut source: Box<dyn Read> = if zst {
            skip = cursor.offset;
            match ruzstd::decoding::StreamingDecoder::new(file) {
                Ok(decoder) => Box::new(decoder),
                Err(_) => return false,
            }
        } else {
            if file.seek(SeekFrom::Start(cursor.offset)).is_err() {
                return false;
            }
            Box::new(file)
        };
        let mut buffer = Vec::new();
        let mut changed = false;
        while let Ok(read) = (&mut source).take(CHUNK).read_to_end(&mut buffer) {
            if read == 0 {
                break;
            }
            if skip > 0 {
                let passed = skip.min(buffer.len() as u64);
                buffer.drain(..passed as usize);
                skip -= passed;
            }
            let complete = memchr::memrchr(b'\n', &buffer).map_or(0, |at| at + 1);
            for line in buffer[..complete].split(|b| *b == b'\n') {
                // Most lines are messages and tool output: only these two kinds are parsed.
                let wanted = memchr::memmem::find(line, b"\"turn_context\"").is_some() || memchr::memmem::find(line, b"\"token_count\"").is_some();
                if wanted && let Ok(value) = serde_json::from_slice::<Value>(line) {
                    changed |= self.count(&value, cursor, cutoff);
                }
            }
            buffer.drain(..complete);
            cursor.offset += complete as u64;
        }
        changed
    }

    fn count(&mut self, line: &Value, cursor: &mut Cursor, cutoff: i64) -> bool {
        let payload = line.get("payload");
        match line.get("type").and_then(Value::as_str) {
            Some("turn_context") => {
                if let Some(model) = payload.and_then(|p| p.get("model")).and_then(Value::as_str) {
                    cursor.model = model.to_owned();
                }
                if let Some(cwd) = payload.and_then(|p| p.get("cwd")).and_then(Value::as_str) {
                    cursor.cwd = cwd.to_owned();
                }
                false
            }
            Some("event_msg") if payload.and_then(|p| p.get("type")).and_then(Value::as_str) == Some("token_count") => {
                let Some(total) = payload.and_then(|p| p.pointer("/info/total_token_usage")) else { return false };
                let field = |name: &str| total.get(name).and_then(Value::as_u64).unwrap_or(0);
                let now = [field("input_tokens"), field("cached_input_tokens"), field("cache_write_input_tokens"), field("output_tokens")];
                let before = std::mem::replace(&mut cursor.total, now);
                // A repeat, or a total that was rewritten lower (a compacted thread): nothing new.
                if now.iter().zip(before).any(|(now, before)| *now < before) || now == before {
                    return false;
                }
                let at = line.get("timestamp").and_then(Value::as_str).and_then(time::parse_iso);
                let Some(at) = at.filter(|at| *at >= cutoff) else { return false };
                let delta = |at: usize| (now[at] - before[at]).min(u32::MAX as u64) as u32;
                // Input counts its cached part too: what was sent anew is the rest.
                let (input, cached, written, output) = (delta(0), delta(1), delta(2), delta(3));
                let mut tokens = [0u32; 5];
                tokens[INPUT] = input.saturating_sub(cached).saturating_sub(written);
                tokens[CACHE_READ] = cached;
                tokens[CACHE_WRITE_5M] = written;
                tokens[OUTPUT] = output;
                let model = if cursor.model.is_empty() { "gpt" } else { cursor.model.as_str() };
                // The request's size, for its price tier: what the event says the last response
                // took, else the rise of the total.
                let prompt = payload.and_then(|p| p.pointer("/info/last_token_usage/input_tokens")).and_then(Value::as_u64).unwrap_or(input as u64);
                self.counts.record(at, model, &cursor.cwd, prompt, tokens);
                true
            }
            _ => false,
        }
    }

    pub fn view(&self, prices: &Prices, now: i64) -> StatsView {
        self.counts.view(prices, now)
    }
}

/// The session files under `dir` changed since `cutoff`, plain or compressed, with their
/// sizes: up to four levels down, year, month and day.
fn collect(dir: &Path, cutoff: i64, depth: usize, out: &mut Vec<(PathBuf, u64)>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else { continue };
        let path = entry.path();
        if kind.is_dir() && depth < 4 {
            collect(&path, cutoff, depth + 1, out);
        } else if kind.is_file() && path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.ends_with(".jsonl") || name.ends_with(".jsonl.zst")) {
            let Ok(meta) = entry.metadata() else { continue };
            let modified = meta.modified().ok().and_then(|at| at.duration_since(UNIX_EPOCH).ok()).map_or(i64::MAX, |d| d.as_secs() as i64);
            if modified >= cutoff {
                out.push((path, meta.len()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_response_counts_once_by_the_rise_of_the_total() {
        let dir = std::env::temp_dir().join(format!("nerda-codex-{}", std::process::id()));
        let day = dir.join("sessions/2026/11/16");
        std::fs::create_dir_all(&day).unwrap();
        let now = time::unix_now();
        let at = |offset: i64| {
            let t = now - offset;
            let days = t.div_euclid(86_400);
            let (y, m, d) = civil(days);
            let s = t.rem_euclid(86_400);
            format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.000Z", s / 3600, s % 3600 / 60, s % 60)
        };
        let count = |offset: i64, input: u64, cached: u64, output: u64| {
            format!(
                r#"{{"timestamp":"{}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":{input},"cached_input_tokens":{cached},"output_tokens":{output},"reasoning_output_tokens":0,"total_tokens":0}},"last_token_usage":{{}}}},"rate_limits":null}}}}"#,
                at(offset)
            )
        };
        let lines = [
            format!(r#"{{"timestamp":"{}","type":"turn_context","payload":{{"turn_id":"t","cwd":"/work/nerda.bar","model":"gpt-5.1-codex"}}}}"#, at(300)),
            count(200, 1000, 400, 50),
            // The same total again, when rate limits arrived: no new tokens.
            count(190, 1000, 400, 50),
            count(100, 2500, 1400, 80),
        ];
        std::fs::write(day.join("rollout-a.jsonl"), lines.join("\n") + "\n").unwrap();
        let mut logs = CodexLogs::new(&dir);
        assert!(logs.scan(now));
        let view = logs.view(&Prices::default(), now);
        let today = &view.ranges[1];
        // 2500 input of which 1400 cached, and 80 output.
        assert_eq!(today.tokens[INPUT], 1100);
        assert_eq!(today.tokens[CACHE_READ], 1400);
        assert_eq!(today.tokens[OUTPUT], 80);
        assert_eq!(today.models[0].name, "gpt-5.1-codex");
        assert_eq!(today.projects.len(), 1);
        assert_eq!((today.projects[0].name.as_str(), today.projects[0].path.as_str()), ("nerda.bar", "/work"));
        assert_eq!(today.projects[0].tokens, today.total());
        assert!(!logs.scan(now), "nothing new on a second scan");
        // Archived, the session moves: the same file under another folder is the same session.
        std::fs::create_dir_all(dir.join("archived_sessions")).unwrap();
        std::fs::rename(day.join("rollout-a.jsonl"), dir.join("archived_sessions/rollout-a.jsonl")).unwrap();
        assert!(!logs.scan(now), "a session that moved is not counted again");
        // And a session that was already archived when the app started counts.
        let mut fresh = CodexLogs::new(&dir);
        assert!(fresh.scan(now));
        assert_eq!(fresh.view(&Prices::default(), now).ranges[1].tokens[OUTPUT], 80);
        // Compressed, as Codex leaves older sessions: nothing new for the app that counted it,
        // and the whole session for one that starts later.
        let plain = std::fs::read(dir.join("archived_sessions/rollout-a.jsonl")).unwrap();
        let packed = ruzstd::encoding::compress_to_vec(plain.as_slice(), ruzstd::encoding::CompressionLevel::Fastest);
        std::fs::write(dir.join("archived_sessions/rollout-a.jsonl.zst"), packed).unwrap();
        std::fs::remove_file(dir.join("archived_sessions/rollout-a.jsonl")).unwrap();
        assert!(!logs.scan(now), "a session that was compressed is not counted again");
        let mut later = CodexLogs::new(&dir);
        assert!(later.scan(now));
        assert_eq!(later.view(&Prices::default(), now).ranges[1].tokens[OUTPUT], 80);
        assert!(!later.scan(now));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The date of a day counted from the Unix epoch.
    fn civil(days: i64) -> (i64, i64, i64) {
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        (yoe + era * 400 + if m <= 2 { 1 } else { 0 }, m, d)
    }
}
