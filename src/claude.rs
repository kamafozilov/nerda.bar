//! The Claude provider: reads Claude Code's login and asks Anthropic for the plan's limits.
//!
//! The login is only ever read. Claude Code rotates its refresh token when it renews the
//! login, so renewing it from here could sign the CLI out; an expired login is reported
//! instead and Claude Code renews it the next time it runs.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::Value;

use crate::model::{ExtraUsage, Limit, Problem, Snapshot};
use crate::time;

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const OAUTH_BETA: &str = "oauth-2025-04-20";
const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";
/// How long to stay away after a 429 that names no `Retry-After`.
const RATE_LIMIT_PAUSE: i64 = 300;
const SESSION_WINDOW: i64 = 5 * 3600;
const WEEK_WINDOW: i64 = 7 * 86_400;

pub struct Credentials {
    access_token: String,
    /// Unix seconds.
    expires_at: Option<i64>,
    /// "Max 20x", "Pro"; empty when the login doesn't say.
    pub plan: String,
}

/// Claude Code's config folders: `CLAUDE_CONFIG_DIR` when set, else `~/.claude` and
/// `~/.config/claude`.
pub fn config_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    if let Some(list) = std::env::var("CLAUDE_CONFIG_DIR").ok().filter(|v| !v.trim().is_empty()) {
        return list
            .split(',')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(|p| p.strip_prefix("~/").map_or_else(|| PathBuf::from(p), |rest| home.join(rest)))
            .collect();
    }
    vec![home.join(".claude"), home.join(".config/claude")]
}

/// Claude Code's login: the Keychain item it keeps on macOS, else `.credentials.json` in its
/// config folder.
pub fn credentials() -> Result<Credentials, Problem> {
    let from_file = || config_dirs().into_iter().find_map(|dir| std::fs::read_to_string(dir.join(".credentials.json")).ok());
    let text = keychain_item().or_else(from_file).ok_or(Problem::SignedOut)?;
    parse_credentials(&text).ok_or(Problem::SignedOut)
}

/// The Keychain item, read through `security` as Claude Code writes it: the item's access list
/// already trusts that tool, so reading it this way never asks for the login password.
fn keychain_item() -> Option<String> {
    let output = Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-w"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim();
    if text.starts_with('{') {
        return Some(text.to_owned());
    }
    // `security -w` prints a value it takes for binary as hex.
    let bytes: Option<Vec<u8>> = (0..text.len() / 2).map(|i| u8::from_str_radix(text.get(2 * i..2 * i + 2)?, 16).ok()).collect();
    String::from_utf8(bytes.filter(|_| text.len() % 2 == 0)?).ok().filter(|json| json.trim_start().starts_with('{'))
}

fn parse_credentials(text: &str) -> Option<Credentials> {
    let root: Value = serde_json::from_str(text).ok()?;
    let oauth = root.get("claudeAiOauth")?;
    let access_token = oauth.get("accessToken")?.as_str()?.trim().to_owned();
    if access_token.is_empty() {
        return None;
    }
    let field = |name: &str| oauth.get(name).and_then(Value::as_str).unwrap_or("");
    Some(Credentials {
        access_token,
        // Milliseconds in the file.
        expires_at: oauth.get("expiresAt").and_then(Value::as_f64).map(|ms| (ms / 1000.0) as i64),
        plan: plan_name(field("subscriptionType"), field("rateLimitTier")),
    })
}

/// "max" with the tier "default_claude_max_20x" is "Max 20x"; other plans are their name.
fn plan_name(subscription: &str, tier: &str) -> String {
    let mut name: String = subscription.trim().to_owned();
    if let Some(first) = name.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    if subscription.eq_ignore_ascii_case("max")
        && let Some(multiple) = tier.split('_').find(|part| part.len() > 1 && part.ends_with('x') && part[..part.len() - 1].bytes().all(|b| b.is_ascii_digit()))
    {
        name = format!("{name} {multiple}");
    }
    name
}

/// Asks Anthropic for the limits of the login's plan.
pub fn fetch(agent: &ureq::Agent) -> Result<Snapshot, Problem> {
    let credentials = credentials()?;
    let now = time::unix_now();
    if credentials.expires_at.is_some_and(|at| at <= now) {
        return Err(Problem::LoginExpired);
    }
    let mut response = agent
        .get(USAGE_URL)
        .header("Authorization", &format!("Bearer {}", credentials.access_token))
        .header("Accept", "application/json")
        .header("anthropic-beta", OAUTH_BETA)
        .header("User-Agent", concat!("nerda-bar/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|e| Problem::Offline(format!("Couldn't reach Claude: {e}.")))?;
    let status = response.status().as_u16();
    match status {
        200 => {}
        401 | 403 => return Err(Problem::LoginExpired),
        429 => {
            let pause = response.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<i64>().ok());
            return Err(Problem::RateLimited { until: now + pause.unwrap_or(RATE_LIMIT_PAUSE).clamp(60, 3600) });
        }
        _ => return Err(Problem::Failed(format!("Claude answered with HTTP {status}."))),
    }
    let body = response.body_mut().read_to_string().map_err(|e| Problem::Offline(format!("Claude's answer was cut short: {e}.")))?;
    crate::debug(|| format!("usage answer: {body}"));
    let root: Value = serde_json::from_str(&body).map_err(|_| Problem::Failed("Claude's answer wasn't JSON.".to_owned()))?;
    Ok(parse_usage(&root, credentials.plan, now))
}

/// The HTTP client the provider uses: no status-as-error, so a 429's headers can be read.
pub fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(20))).http_status_as_error(false).build().into()
}

/// A reset time: ISO 8601, or a number of seconds or milliseconds.
fn reset_time(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::String(text) => time::parse_iso(text),
        Value::Number(number) => number.as_f64().map(|n| if n < 1e10 { n as i64 } else { (n / 1000.0) as i64 }),
        _ => None,
    }
}

fn parse_usage(root: &Value, plan: String, now: i64) -> Snapshot {
    let window = |key: &str, id: &str, label: &str, seconds: i64, scoped: bool| {
        let node = root.get(key).filter(|n| n.is_object())?;
        let used = node.get("utilization").and_then(Value::as_f64)?;
        Some(Limit { id: id.to_owned(), label: label.to_owned(), used: used.clamp(0.0, 100.0), resets_at: reset_time(node.get("resets_at")), window: seconds, scoped })
    };
    let mut limits: Vec<Limit> = [window("five_hour", "session", "Session", SESSION_WINDOW, false), window("seven_day", "weekly", "Weekly", WEEK_WINDOW, false)]
        .into_iter()
        .flatten()
        .collect();

    // Per-model weekly limits: the `limits` list where the API sends one, else the older keys.
    let mut scoped: Vec<Limit> = Vec::new();
    for entry in root.get("limits").and_then(Value::as_array).into_iter().flatten() {
        if entry.get("kind").and_then(Value::as_str) != Some("weekly_scoped") {
            continue;
        }
        let model = entry.pointer("/scope/model");
        let name = model.and_then(|m| m.get("display_name")).and_then(Value::as_str).unwrap_or("").trim();
        let model_id = model.and_then(|m| m.get("id")).and_then(Value::as_str).unwrap_or("");
        let all_models = name.eq_ignore_ascii_case("all models") || model_id == "all-models" || model_id.ends_with("-all-models");
        let Some(used) = entry.get("percent").and_then(Value::as_f64) else { continue };
        if name.is_empty() || all_models {
            continue;
        }
        let id = format!("weekly-{}", name.to_lowercase().replace(' ', "-"));
        if scoped.iter().all(|l| l.id != id) {
            scoped.push(Limit { id, label: format!("{name} weekly"), used: used.clamp(0.0, 100.0), resets_at: reset_time(entry.get("resets_at")), window: WEEK_WINDOW, scoped: true });
        }
    }
    if scoped.is_empty() {
        scoped.extend(window("seven_day_opus", "weekly-opus", "Opus weekly", WEEK_WINDOW, true));
        scoped.extend(window("seven_day_sonnet", "weekly-sonnet", "Sonnet weekly", WEEK_WINDOW, true));
    }
    limits.extend(scoped);

    let extra = root.get("extra_usage").filter(|e| e.get("is_enabled").and_then(Value::as_bool) == Some(true)).map(|e| {
        // Credits come in the currency's minor unit.
        let scale = 10f64.powi(e.get("decimal_places").and_then(Value::as_i64).unwrap_or(2) as i32);
        let amount = |key: &str| e.get(key).and_then(Value::as_f64).map(|v| v / scale);
        ExtraUsage {
            used: amount("used_credits").unwrap_or(0.0),
            limit: amount("monthly_limit").filter(|limit| *limit > 0.0),
            currency: e.get("currency").and_then(Value::as_str).unwrap_or("USD").to_owned(),
        }
    });

    Snapshot { plan, limits, extra, fetched_at: now }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plan_is_named_from_the_login() {
        assert_eq!(plan_name("max", "default_claude_max_20x"), "Max 20x");
        assert_eq!(plan_name("max", "default_claude_max_5x"), "Max 5x");
        assert_eq!(plan_name("max", ""), "Max");
        assert_eq!(plan_name("pro", "default_claude_ai"), "Pro");
        assert_eq!(plan_name("", ""), "");
    }

    #[test]
    fn a_login_parses_and_an_empty_one_does_not() {
        let login = parse_credentials(r#"{"claudeAiOauth":{"accessToken":" abc ","refreshToken":"r","expiresAt":1794478830000,"subscriptionType":"max","rateLimitTier":"default_claude_max_20x"},"mcpOAuth":{}}"#).unwrap();
        assert_eq!(login.access_token, "abc");
        assert_eq!(login.expires_at, Some(1_794_478_830));
        assert_eq!(login.plan, "Max 20x");
        assert!(parse_credentials(r#"{"claudeAiOauth":{"accessToken":""}}"#).is_none());
        assert!(parse_credentials(r#"{"mcpOAuth":{}}"#).is_none());
    }

    #[test]
    fn usage_parses_into_limits() {
        let root: Value = serde_json::from_str(
            r#"{
                "five_hour": {"utilization": 24.0, "resets_at": "2026-11-12T10:20:30.000000+00:00"},
                "seven_day": {"utilization": 41.5, "resets_at": 1794478830},
                "seven_day_opus": null,
                "seven_day_sonnet": {"utilization": 3.0, "resets_at": null},
                "extra_usage": {"is_enabled": true, "monthly_limit": 5000, "used_credits": 1240.0, "currency": "USD"}
            }"#,
        )
        .unwrap();
        let snapshot = parse_usage(&root, "Max 20x".to_owned(), 100);
        let ids: Vec<&str> = snapshot.limits.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, ["session", "weekly", "weekly-sonnet"]);
        assert_eq!(snapshot.limits[0].used, 24.0);
        assert_eq!(snapshot.limits[0].resets_at, Some(1_794_478_830));
        assert_eq!(snapshot.limits[1].resets_at, Some(1_794_478_830));
        assert_eq!(snapshot.limits[2].resets_at, None);
        assert!(snapshot.limits[2].scoped && !snapshot.limits[0].scoped);
        assert_eq!(snapshot.extra, Some(ExtraUsage { used: 12.4, limit: Some(50.0), currency: "USD".to_owned() }));
    }

    #[test]
    fn scoped_limits_come_from_the_list_when_it_is_there() {
        let root: Value = serde_json::from_str(
            r#"{
                "five_hour": {"utilization": 10, "resets_at": null},
                "seven_day_sonnet": {"utilization": 3.0, "resets_at": null},
                "limits": [
                    {"kind": "weekly_scoped", "percent": 12, "resets_at": "2026-11-12T10:20:30Z", "scope": {"model": {"id": "x-all-models", "display_name": "All models"}}},
                    {"kind": "weekly_scoped", "percent": 55, "resets_at": "2026-11-12T10:20:30Z", "scope": {"model": {"id": "fable", "display_name": "Fable"}}},
                    {"kind": "session", "percent": 10}
                ],
                "extra_usage": {"is_enabled": false}
            }"#,
        )
        .unwrap();
        let snapshot = parse_usage(&root, String::new(), 100);
        let ids: Vec<&str> = snapshot.limits.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, ["session", "weekly-fable"]);
        assert_eq!(snapshot.limits[1].label, "Fable weekly");
        assert_eq!(snapshot.limits[1].used, 55.0);
        assert_eq!(snapshot.extra, None);
    }
}
