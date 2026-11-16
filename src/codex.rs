//! The Codex provider: reads the Codex CLI's ChatGPT login and asks ChatGPT for the plan's
//! limits, and for the limit resets the plan has been given.
//!
//! The login is only ever read. Its refresh token is single-use, so renewing it from here
//! would sign the CLI out; an expired login is reported instead and the CLI renews it the next
//! time it runs.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde_json::Value;

use crate::model::{Limit, Problem, Snapshot};
use crate::time;

const BASE: &str = "https://chatgpt.com/backend-api";
const KEYCHAIN_SERVICE: &str = "Codex Auth";
const RATE_LIMIT_PAUSE: i64 = 300;
const WEEK: i64 = 7 * 86_400;

/// `CODEX_HOME`, else `~/.codex`.
pub fn home() -> PathBuf {
    match std::env::var_os("CODEX_HOME").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".codex"),
    }
}

pub struct Credentials {
    access_token: String,
    account_id: Option<String>,
}

/// The CLI's login: `auth.json` in its home, else the Keychain item it keeps when set to.
pub fn credentials() -> Result<Credentials, Problem> {
    let text = std::fs::read_to_string(home().join("auth.json")).ok().or_else(|| keyring_store().then(keychain_item).flatten()).ok_or(Problem::SignedOut)?;
    parse_credentials(&text).ok_or(Problem::SignedOut)
}

/// Whether the CLI is set to keep its login in the Keychain: only then is the Keychain asked,
/// so a Mac without Codex never sees a Keychain prompt on its behalf.
fn keyring_store() -> bool {
    std::fs::read_to_string(home().join("config.toml")).is_ok_and(|text| {
        text.lines().any(|line| {
            let line = line.trim();
            line.starts_with("cli_auth_credentials_store") && (line.contains("\"keyring\"") || line.contains("\"auto\""))
        })
    })
}

/// The Keychain item, by service alone: its account names the CLI's home by a hash.
fn keychain_item() -> Option<String> {
    let output = Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-w"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    (output.status.success() && text.trim_start().starts_with('{')).then(|| text.trim().to_owned())
}

fn parse_credentials(text: &str) -> Option<Credentials> {
    let root: Value = serde_json::from_str(text).ok()?;
    let tokens = root.get("tokens")?;
    let access_token = tokens.get("access_token")?.as_str()?.trim().to_owned();
    if access_token.is_empty() {
        return None;
    }
    let account_id = tokens.get("account_id").and_then(Value::as_str).filter(|id| !id.is_empty()).map(str::to_owned);
    Some(Credentials { access_token, account_id })
}

fn get(agent: &ureq::Agent, credentials: &Credentials, path: &str) -> Result<Value, Problem> {
    let now = time::unix_now();
    let mut request = agent
        .get(&format!("{BASE}{path}"))
        .header("Authorization", &format!("Bearer {}", credentials.access_token))
        .header("Accept", "application/json")
        .header("User-Agent", concat!("nerda-bar/", env!("CARGO_PKG_VERSION")));
    if let Some(account) = &credentials.account_id {
        request = request.header("chatgpt-account-id", account);
    }
    let mut response = request.call().map_err(|e| Problem::Offline(format!("Couldn't reach ChatGPT: {e}.")))?;
    match response.status().as_u16() {
        200 => {}
        401 | 403 => return Err(Problem::LoginExpired),
        429 => {
            let pause = response.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<i64>().ok());
            return Err(Problem::RateLimited { until: now + pause.unwrap_or(RATE_LIMIT_PAUSE).clamp(60, 3600) });
        }
        status => return Err(Problem::Failed(format!("The server answered {status}."))),
    }
    let body = response.body_mut().read_to_string().map_err(|e| Problem::Offline(format!("ChatGPT's answer was cut short: {e}.")))?;
    crate::debug(|| format!("codex {path}: {body}"));
    serde_json::from_str(&body).map_err(|_| Problem::Failed("ChatGPT's answer wasn't JSON.".to_owned()))
}

/// The plan's limits, with the resets it has been given when it has any.
pub fn fetch(agent: &ureq::Agent) -> Result<Snapshot, Problem> {
    let credentials = credentials()?;
    let now = time::unix_now();
    let root = get(agent, &credentials, "/wham/usage")?;
    let mut snapshot = parse_usage(&root, now);
    if snapshot.resets.as_ref().is_some_and(|resets| resets.available > 0)
        && let Ok(list) = get(agent, &credentials, "/wham/rate-limit-reset-credits")
    {
        snapshot.resets = Some(parse_resets(&list));
    }
    Ok(snapshot)
}

/// The plan's name as ChatGPT sells it.
fn plan_name(plan: &str) -> String {
    let name = match plan {
        "plus" => "Plus",
        "pro" => "Pro",
        "prolite" => "Pro Lite",
        "promax" => "Pro Max",
        "team" => "Team",
        "business" => "Business",
        "enterprise" => "Enterprise",
        "edu" => "Edu",
        "go" => "Go",
        "free" => "Free",
        _ => return String::new(),
    };
    format!("ChatGPT {name}")
}

fn window(node: Option<&Value>, id: &str, label: &str, scoped: bool, now: i64) -> Option<Limit> {
    let node = node.filter(|n| n.is_object())?;
    let used = node.get("used_percent").and_then(Value::as_f64)?;
    let window = node.get("limit_window_seconds").and_then(Value::as_i64).filter(|w| *w > 0).unwrap_or(WEEK);
    let resets_at = node.get("reset_at").and_then(Value::as_i64).filter(|at| *at > 0).or_else(|| node.get("reset_after_seconds").and_then(Value::as_i64).map(|after| now + after));
    Some(Limit { id: id.to_owned(), label: label.to_owned(), used: used.clamp(0.0, 100.0), resets_at, window, scoped })
}

/// "5-hour" for a window of five hours, "Weekly" for a week.
fn window_label(seconds: i64) -> String {
    match seconds {
        WEEK => "Weekly".to_owned(),
        86_400 => "Daily".to_owned(),
        s if s % 3600 == 0 => format!("{}-hour", s / 3600),
        s => format!("{}-minute", s / 60),
    }
}

fn parse_usage(root: &Value, now: i64) -> Snapshot {
    let mut limits = Vec::new();
    if let Some(rate) = root.get("rate_limit") {
        let label = |key: &str| rate.pointer(&format!("/{key}/limit_window_seconds")).and_then(Value::as_i64).map_or_else(|| "Weekly".to_owned(), window_label);
        limits.extend(window(rate.get("primary_window"), "session", &label("primary_window"), false, now));
        limits.extend(window(rate.get("secondary_window"), "weekly", &label("secondary_window"), false, now));
    }
    // Limits on one model or feature, each as its weekly window.
    for extra in root.get("additional_rate_limits").and_then(Value::as_array).into_iter().flatten() {
        let name = extra.get("limit_name").and_then(Value::as_str).unwrap_or("").trim();
        let Some(rate) = extra.get("rate_limit").filter(|_| !name.is_empty()) else { continue };
        let id = format!("weekly-{}", name.to_lowercase().replace(' ', "-"));
        let node = rate.get("secondary_window").filter(|n| n.is_object()).or_else(|| rate.get("primary_window"));
        limits.extend(window(node, &id, &format!("{name} weekly"), true, now));
    }
    let resets = root.pointer("/rate_limit_reset_credits/available_count").and_then(Value::as_i64).filter(|n| *n > 0).map(|n| crate::model::Resets { available: n as u32, credits: Vec::new() });
    Snapshot { plan: plan_name(root.get("plan_type").and_then(Value::as_str).unwrap_or("")), limits, extra: None, fetched_at: now, resets }
}

fn parse_resets(root: &Value) -> crate::model::Resets {
    let credits: Vec<crate::model::ResetCredit> = root
        .get("credits")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|credit| credit.get("status").and_then(Value::as_str).is_none_or(|status| status == "available"))
        .filter_map(|credit| {
            Some(crate::model::ResetCredit {
                id: credit.get("id")?.as_str()?.to_owned(),
                title: credit.get("title").and_then(Value::as_str).unwrap_or("Limit reset").to_owned(),
                description: credit.get("description").and_then(Value::as_str).map(str::to_owned),
                expires_at: credit.get("expires_at").and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(time::parse_iso))),
            })
        })
        .collect();
    let available = root.get("available_count").and_then(Value::as_i64).map_or(credits.len() as u32, |n| n.max(0) as u32);
    crate::model::Resets { available, credits }
}

/// What spending a reset came to.
#[derive(Clone, Debug, PartialEq)]
pub enum Redeemed {
    Reset,
    NothingToReset,
    /// ChatGPT said no: nothing was spent.
    Failed(String),
    /// No answer came back, so whether it was spent is not known; the limits will tell.
    Unknown(String),
}

/// Spends the reset `credit`: this can't be undone, so it is only ever called after the person
/// confirmed it in the panel. The request carries a fresh id, so a retry is never a second spend.
pub fn redeem(agent: &ureq::Agent, credit: &str, request_id: &str) -> Redeemed {
    let Ok(credentials) = credentials() else { return Redeemed::Failed("Codex isn't signed in.".to_owned()) };
    let body = serde_json::json!({ "redeem_request_id": request_id, "credit_id": credit }).to_string();
    let mut request = agent
        .post(&format!("{BASE}/wham/rate-limit-reset-credits/consume"))
        .header("Authorization", &format!("Bearer {}", credentials.access_token))
        .header("Content-Type", "application/json")
        .header("User-Agent", concat!("nerda-bar/", env!("CARGO_PKG_VERSION")));
    if let Some(account) = &credentials.account_id {
        request = request.header("chatgpt-account-id", account);
    }
    let mut response = match request.send(body.as_bytes()) {
        Ok(response) => response,
        Err(e) => return Redeemed::Unknown(format!("Couldn't reach ChatGPT: {e}.")),
    };
    let status = response.status().as_u16();
    match status {
        200 => {}
        // A server error may come after the reset was spent.
        500.. => return Redeemed::Unknown(format!("The server answered {status}.")),
        _ => return Redeemed::Failed(format!("The server answered {status}.")),
    }
    let answer: Value = response.body_mut().read_to_string().ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or(Value::Null);
    match answer.get("code").and_then(Value::as_str) {
        Some("reset" | "already_redeemed") => Redeemed::Reset,
        Some("nothing_to_reset") => Redeemed::NothingToReset,
        Some("no_credit") => Redeemed::Failed("That reset is no longer available.".to_owned()),
        _ => Redeemed::Failed("ChatGPT's answer wasn't understood.".to_owned()),
    }
}

/// A random version 4 UUID, for a request that must not repeat.
pub fn request_id() -> String {
    let mut bytes = [0u8; 16];
    // SAFETY: fills the buffer it is given.
    unsafe { libc::arc4random_buf(bytes.as_mut_ptr() as *mut libc::c_void, bytes.len()) };
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_login_parses_from_the_cli_file() {
        let login = parse_credentials(r#"{"auth_mode":"chatgpt","OPENAI_API_KEY":null,"tokens":{"id_token":"x","access_token":" tok ","refresh_token":"r","account_id":"acc"},"last_refresh":"2026-11-01T00:00:00Z"}"#).unwrap();
        assert_eq!(login.access_token, "tok");
        assert_eq!(login.account_id.as_deref(), Some("acc"));
        assert!(parse_credentials(r#"{"OPENAI_API_KEY":"sk-x","tokens":null}"#).is_none());
    }

    #[test]
    fn usage_parses_into_the_five_hour_and_weekly_limits() {
        let root: Value = serde_json::from_str(
            r#"{"plan_type":"pro","rate_limit":{"allowed":true,"limit_reached":false,
                "primary_window":{"used_percent":29,"limit_window_seconds":18000,"reset_after_seconds":11520,"reset_at":1794600000},
                "secondary_window":{"used_percent":36,"limit_window_seconds":604800,"reset_after_seconds":360000,"reset_at":1794900000}},
                "credits":{"has_credits":false,"unlimited":false,"balance":"0"},
                "rate_limit_reset_credits":{"available_count":1}}"#,
        )
        .unwrap();
        let snapshot = parse_usage(&root, 1_794_588_480);
        assert_eq!(snapshot.plan, "ChatGPT Pro");
        let shown: Vec<(&str, &str, f64)> = snapshot.limits.iter().map(|l| (l.id.as_str(), l.label.as_str(), l.used)).collect();
        assert_eq!(shown, [("session", "5-hour", 29.0), ("weekly", "Weekly", 36.0)]);
        assert_eq!(snapshot.limits[0].window, 18_000);
        assert_eq!(snapshot.limits[1].resets_at, Some(1_794_900_000));
        assert_eq!(snapshot.resets.map(|r| r.available), Some(1));
    }

    #[test]
    fn resets_list_only_the_available_ones() {
        let root: Value = serde_json::from_str(
            r#"{"credits":[{"id":"c1","reset_type":"codex_rate_limits","status":"available","granted_at":1,"expires_at":1796000000,"title":"Full reset (Weekly + 5 hr)"},
                {"id":"c2","status":"redeemed","title":"Old"}],"available_count":1,"total_earned_count":2}"#,
        )
        .unwrap();
        let resets = parse_resets(&root);
        assert_eq!(resets.available, 1);
        assert_eq!(resets.credits.len(), 1);
        assert_eq!(resets.credits[0].title, "Full reset (Weekly + 5 hr)");
        assert_eq!(resets.credits[0].expires_at, Some(1_796_000_000));
    }

    #[test]
    fn request_ids_are_version_four_uuids() {
        let id = request_id();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        assert_ne!(id, request_id());
    }
}
