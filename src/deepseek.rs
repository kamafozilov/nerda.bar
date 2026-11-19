//! The DeepSeek provider: the API account's balance, and what it was spent on day by day.
//!
//! DeepSeek reports only the balance, so the spend is read from how the balance falls between
//! refreshes, and a rise is a top-up. The key is read from the Keychain item the panel tells
//! the person how to add, or from `DEEPSEEK_API_KEY` when the app is started from a shell.

use std::path::Path;
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::Problem;
use crate::time;

const BALANCE_URL: &str = "https://api.deepseek.com/user/balance";
/// Where the key is kept: `security add-generic-password -s "Nerda Bar" -a deepseek -w`.
pub const KEYCHAIN_SERVICE: &str = "Nerda Bar";
pub const KEYCHAIN_ACCOUNT: &str = "deepseek";
const FILE: &str = "deepseek.json";
/// Days of balance history kept.
const KEEP_DAYS: i64 = 31;

/// The account's balance at one refresh.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Balance {
    pub currency: String,
    pub total: f64,
    pub granted: f64,
    pub topped_up: f64,
    /// Whether the balance still covers API calls.
    pub available: bool,
    pub fetched_at: i64,
}

/// The balance as each refresh found it, kept on disk for the spend per day.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Ledger {
    pub readings: Vec<(i64, f64)>,
    pub last: Option<Balance>,
}

/// A day's spend and top-ups, from the falls and rises of the balance.
#[derive(Clone, Debug, PartialEq)]
pub struct Day {
    /// Unix seconds of the local day's start.
    pub start: i64,
    pub spent: f64,
    pub topped_up: f64,
}

impl Ledger {
    pub fn load(dir: &Path) -> Ledger {
        std::fs::read_to_string(dir.join(FILE)).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) {
        if std::fs::create_dir_all(dir).is_ok()
            && let Ok(text) = serde_json::to_string(self)
        {
            let _ = std::fs::write(dir.join(FILE), text);
        }
    }

    pub fn record(&mut self, balance: Balance) {
        let at = balance.fetched_at;
        self.readings.push((at, balance.total));
        self.readings.retain(|(t, _)| at - t <= KEEP_DAYS * 86_400);
        self.last = Some(balance);
    }

    /// The last `days` local days, oldest first, today last.
    pub fn days(&self, now: i64, days: i32) -> Vec<Day> {
        let today = time::local(now).day;
        let mut out: Vec<Day> = (0..days).map(|at| Day { start: time::local_hour_start(today - days + 1 + at, 0), spent: 0.0, topped_up: 0.0 }).collect();
        for pair in self.readings.windows(2) {
            let ((_, before), (at, after)) = (pair[0], pair[1]);
            let day = time::local(at).day - (today - days + 1);
            let Some(slot) = usize::try_from(day).ok().and_then(|day| out.get_mut(day)) else { continue };
            if after < before {
                slot.spent += before - after;
            } else {
                slot.topped_up += after - before;
            }
        }
        out
    }

    /// The average spend a day over the last week, for how long the balance lasts.
    pub fn daily_spend(&self, now: i64) -> Option<f64> {
        let days = self.days(now, 7);
        let spent: f64 = days.iter().map(|day| day.spent).sum();
        let first = self.readings.first()?.0;
        // Only as many days as there are readings for.
        let span = ((now - first.max(now - 7 * 86_400)) as f64 / 86_400.0).max(1.0);
        (spent > 0.0).then(|| spent / span)
    }
}

/// The API key: the Keychain item, else the environment.
pub fn key() -> Option<String> {
    let output = Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-a", KEYCHAIN_ACCOUNT, "-w"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok();
    output
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|key| key.trim().to_owned())
        .filter(|key| !key.is_empty())
        .or_else(|| std::env::var("DEEPSEEK_API_KEY").ok().filter(|key| !key.trim().is_empty()))
}

/// Saves `key` as the Keychain item [`key`] reads, replacing one there. The `security` tool
/// reads the command from its input rather than its arguments, so the key never shows in a
/// process list, and the item it makes is one it can read back without asking.
pub fn save_key(key: &str) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_graphic() && c != '"' && c != '\\') {
        return Err("That doesn't look like a DeepSeek key.".to_owned());
    }
    let mut child = Command::new("/usr/bin/security")
        .arg("-i")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Couldn't open the Keychain: {e}."))?;
    let line = format!("add-generic-password -U -s \"{KEYCHAIN_SERVICE}\" -a \"{KEYCHAIN_ACCOUNT}\" -w \"{key}\"\n");
    if let Some(mut input) = child.stdin.take() {
        use std::io::Write;
        input.write_all(line.as_bytes()).map_err(|e| format!("Couldn't reach the Keychain: {e}."))?;
    }
    let output = child.wait_with_output().map_err(|e| format!("Couldn't reach the Keychain: {e}."))?;
    let said = String::from_utf8_lossy(&output.stderr);
    if output.status.success() && said.trim().is_empty() { Ok(()) } else { Err("The Keychain didn't save the key.".to_owned()) }
}

pub fn fetch(agent: &ureq::Agent) -> Result<Balance, Problem> {
    let key = key().ok_or(Problem::SignedOut)?;
    let now = time::unix_now();
    let mut response = agent
        .get(BALANCE_URL)
        .header("Authorization", &format!("Bearer {key}"))
        .header("Accept", "application/json")
        .header("User-Agent", concat!("nerda-bar/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|e| Problem::Offline(format!("Couldn't reach DeepSeek: {e}.")))?;
    match response.status().as_u16() {
        200 => {}
        401 | 403 => return Err(Problem::LoginExpired),
        429 => return Err(Problem::RateLimited { until: now + 300 }),
        status => return Err(Problem::Failed(format!("The server answered {status}."))),
    }
    let body = response.body_mut().read_to_string().map_err(|e| Problem::Offline(format!("DeepSeek's answer was cut short: {e}.")))?;
    let root: Value = serde_json::from_str(&body).map_err(|_| Problem::Failed("DeepSeek's answer wasn't JSON.".to_owned()))?;
    parse(&root, now).ok_or_else(|| Problem::Failed("DeepSeek's answer had no balance.".to_owned()))
}

fn parse(root: &Value, now: i64) -> Option<Balance> {
    let infos = root.get("balance_infos")?.as_array()?;
    // One balance per currency; dollars where there are both.
    let info = infos.iter().find(|i| i.get("currency").and_then(Value::as_str) == Some("USD")).or_else(|| infos.first())?;
    let amount = |key: &str| info.get(key).and_then(|v| v.as_str().and_then(|s| s.parse().ok()).or_else(|| v.as_f64())).unwrap_or(0.0);
    Some(Balance {
        currency: info.get("currency").and_then(Value::as_str).unwrap_or("USD").to_owned(),
        total: amount("total_balance"),
        granted: amount("granted_balance"),
        topped_up: amount("topped_up_balance"),
        available: root.get("is_available").and_then(Value::as_bool).unwrap_or(true),
        fetched_at: now,
    })
}

/// An amount in the balance's currency: "$4.20", "¥110.00".
pub fn money(amount: f64, currency: &str) -> String {
    match currency {
        "USD" => format!("${amount:.2}"),
        "CNY" => format!("¥{amount:.2}"),
        other => format!("{amount:.2} {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_balance_parses_from_its_strings() {
        let root: Value = serde_json::from_str(r#"{"is_available":true,"balance_infos":[{"currency":"USD","total_balance":"4.20","granted_balance":"0.20","topped_up_balance":"4.00"}]}"#).unwrap();
        let balance = parse(&root, 5).unwrap();
        assert_eq!((balance.total, balance.granted, balance.topped_up), (4.2, 0.2, 4.0));
        assert_eq!(money(balance.total, &balance.currency), "$4.20");
    }

    #[test]
    fn falls_are_spend_and_rises_are_top_ups() {
        let now = time::unix_now();
        let mut ledger = Ledger::default();
        for (ago, total) in [(3 * 3600, 5.0), (2 * 3600, 4.6), (3600, 9.6), (60, 9.2)] {
            ledger.readings.push((now - ago, total));
        }
        let today = ledger.days(now, 7).pop().unwrap();
        // Readings within the last few hours may straddle midnight; across both days the sums hold.
        let all = ledger.days(now, 7);
        let spent: f64 = all.iter().map(|d| d.spent).sum();
        let topped: f64 = all.iter().map(|d| d.topped_up).sum();
        assert!((spent - 0.8).abs() < 1e-9 && (topped - 5.0).abs() < 1e-9, "{spent} {topped}");
        assert!(today.start <= now);
    }
}
