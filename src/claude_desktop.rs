//! What claude.ai tells with the claude.ai session the Claude desktop app keeps in its cookies:
//! when the plan renews, which only claude.ai knows, and the plan's limits when Claude Code's
//! own login has expired. Claude Code sessions run from the desktop app use the app's login, so
//! the CLI's login there can stay expired for days while the app's session stays fresh.
//!
//! The cookies are encrypted with a key the app keeps in the Keychain item "Claude Safe
//! Storage", so macOS asks once whether it may be read. A refusal is kept until Nerda Bar
//! quits: it is asked at most once a launch. The session is only read and only sent to
//! claude.ai. The request goes through URLSession, as claude.ai turns away other HTTP stacks.

use std::process::{Command, Stdio};
use std::sync::{Mutex, mpsc};
use std::time::Duration;

use block2::RcBlock;
use objc2_foundation::{NSData, NSError, NSHTTPURLResponse, NSMutableURLRequest, NSString, NSURL, NSURLResponse, NSURLSession, NSURLSessionConfiguration};
use serde_json::Value;

use crate::model::Subscription;
use crate::time;

const KEYCHAIN_SERVICE: &str = "Claude Safe Storage";
/// Seconds from 1601-01-01, which Chromium counts cookie times from, to 1970-01-01.
const CHROMIUM_EPOCH: i64 = 11_644_473_600;
/// From version 24 of Chromium's cookie store a value starts with a hash of its host.
const HOST_HASH_FROM: u32 = 24;
const TIMEOUT: Duration = Duration::from_secs(20);

enum Key {
    Unread,
    Read([u8; 16]),
    Refused,
}

static KEY: Mutex<Key> = Mutex::new(Key::Unread);

/// The claude.ai session the desktop app is signed in with, or `None` when it isn't installed,
/// isn't signed in, or its cookies can't be opened.
pub fn session() -> Option<String> {
    let (version, encrypted) = session_cookie()?;
    let key = key()?;
    let plain = decrypt(&key, encrypted.strip_prefix(b"v10")?)?;
    let value = if version >= HOST_HASH_FROM { plain.get(32..)? } else { &plain[..] };
    String::from_utf8(value.to_vec()).ok().filter(|v| v.starts_with("sk-ant-"))
}

/// The cookie store's version and the session cookie's encrypted value, read without the
/// Keychain, so a Mac without a signed-in desktop app is never asked for the key.
fn session_cookie() -> Option<(u32, Vec<u8>)> {
    let home = std::env::var_os("HOME")?;
    let store = std::path::Path::new(&home).join("Library/Application Support/Claude/Cookies");
    if !store.is_file() {
        return None;
    }
    let now = (time::unix_now() + CHROMIUM_EPOCH) * 1_000_000;
    let query = format!(
        "select m.value, hex(c.encrypted_value) from cookies c, meta m where m.key = 'version' and c.host_key in ('.claude.ai', 'claude.ai') \
         and c.name = 'sessionKey' and c.expires_utc > {now} order by c.expires_utc desc limit 1"
    );
    // Immutable: the app may hold the store open, and nothing here writes to it.
    let output = Command::new("/usr/bin/sqlite3")
        .args(["-readonly", &format!("file:{}?immutable=1", store.display()), &query])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    let (version, hex) = text.trim().split_once('|')?;
    Some((version.parse().ok()?, hex_bytes(hex)?))
}

fn hex_bytes(hex: &str) -> Option<Vec<u8>> {
    hex.len().is_multiple_of(2).then_some(())?;
    (0..hex.len()).step_by(2).map(|at| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok()).collect()
}

/// The cookies' AES key, derived from the Keychain item's password as Chromium derives it.
fn key() -> Option<[u8; 16]> {
    let mut state = KEY.lock().unwrap_or_else(|e| e.into_inner());
    match *state {
        Key::Read(key) => return Some(key),
        Key::Refused => return None,
        Key::Unread => {}
    }
    let password = Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-w"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty());
    let Some(password) = password else {
        *state = Key::Refused;
        return None;
    };
    let mut key = [0u8; 16];
    const SALT: &[u8] = b"saltysalt";
    // SAFETY: every pointer is valid for the length passed with it.
    let status = unsafe { CCKeyDerivationPBKDF(PBKDF2, password.as_ptr(), password.len(), SALT.as_ptr(), SALT.len(), PRF_SHA1, 1003, key.as_mut_ptr(), key.len()) };
    *state = if status == 0 { Key::Read(key) } else { Key::Refused };
    (status == 0).then_some(key)
}

/// AES-128-CBC with PKCS#7 padding and Chromium's IV of sixteen spaces.
fn decrypt(key: &[u8; 16], data: &[u8]) -> Option<Vec<u8>> {
    let iv = [b' '; 16];
    let mut out = vec![0u8; data.len() + 16];
    let mut moved = 0usize;
    // SAFETY: every pointer is valid for the length passed with it.
    let status = unsafe { CCCrypt(DECRYPT, AES, PKCS7, key.as_ptr(), key.len(), iv.as_ptr(), data.as_ptr(), data.len(), out.as_mut_ptr(), out.len(), &mut moved) };
    (status == 0).then(|| {
        out.truncate(moved);
        out
    })
}

const PBKDF2: u32 = 2;
const PRF_SHA1: u32 = 1;
const DECRYPT: u32 = 1;
const AES: u32 = 0;
const PKCS7: u32 = 1;

// CommonCrypto, part of libSystem.
unsafe extern "C" {
    fn CCKeyDerivationPBKDF(algorithm: u32, password: *const u8, password_len: usize, salt: *const u8, salt_len: usize, prf: u32, rounds: u32, derived: *mut u8, derived_len: usize) -> i32;
    fn CCCrypt(op: u32, alg: u32, options: u32, key: *const u8, key_len: usize, iv: *const u8, data_in: *const u8, data_in_len: usize, data_out: *mut u8, data_out_available: usize, data_out_moved: *mut usize) -> i32;
}

/// When the plan of `org` renews or ends, as claude.ai's billing page reads it. `Ok(None)`:
/// the plan has no date, as a free one.
pub fn subscription(org: &str, session: &str) -> Result<Option<Subscription>, String> {
    let (status, body) = get(&format!("https://claude.ai/api/organizations/{org}/subscription_details"), &format!("sessionKey={session}"))?;
    if status != 200 {
        return Err(format!("claude.ai answered {status}"));
    }
    let root: Value = serde_json::from_slice(&body).map_err(|_| "claude.ai's answer wasn't JSON".to_owned())?;
    Ok(parse_subscription(&root))
}

fn parse_subscription(root: &Value) -> Option<Subscription> {
    let at = |key: &str| root.get(key).and_then(Value::as_str).and_then(time::parse_iso);
    match at("plan_ending_at") {
        Some(until) => Some(Subscription { until, renews: Some(false) }),
        None => at("next_charge_at").map(|until| Subscription { until, renews: Some(true) }),
    }
}

/// The organizations the session belongs to that can chat, in claude.ai's order.
pub fn organizations(session: &str) -> Result<Vec<String>, String> {
    let (status, body) = get("https://claude.ai/api/organizations", &format!("sessionKey={session}"))?;
    if status != 200 {
        return Err(format!("claude.ai answered {status}"));
    }
    let root: Value = serde_json::from_slice(&body).map_err(|_| "claude.ai's answer wasn't JSON".to_owned())?;
    Ok(chat_organizations(&root))
}

fn chat_organizations(root: &Value) -> Vec<String> {
    let orgs = root.as_array().map(Vec::as_slice).unwrap_or_default();
    let can_chat = |org: &Value| org.get("capabilities").and_then(Value::as_array).is_some_and(|c| c.iter().any(|c| c.as_str() == Some("chat")));
    orgs.iter().filter(|org| can_chat(org)).filter_map(|org| org.get("uuid").and_then(Value::as_str)).filter(|id| !id.is_empty()).map(str::to_owned).collect()
}

/// The limits of `org`'s plan, in the shape Anthropic's OAuth usage answer has.
pub fn usage(org: &str, session: &str) -> Result<Value, String> {
    let (status, body) = get(&format!("https://claude.ai/api/organizations/{org}/usage"), &format!("sessionKey={session}"))?;
    if status != 200 {
        return Err(format!("claude.ai answered {status}"));
    }
    crate::debug(|| format!("claude.ai usage answer: {}", String::from_utf8_lossy(&body)));
    serde_json::from_slice(&body).map_err(|_| "claude.ai's answer wasn't JSON".to_owned())
}

/// A GET through an ephemeral URLSession, so nothing it is sent or answered is kept: the
/// status and the body.
fn get(url: &str, cookie: &str) -> Result<(isize, Vec<u8>), String> {
    objc2::rc::autoreleasepool(|_| {
        let url = NSURL::URLWithString(&NSString::from_str(url)).ok_or("not a URL")?;
        let request = NSMutableURLRequest::requestWithURL(&url);
        request.setTimeoutInterval(TIMEOUT.as_secs_f64());
        request.setHTTPShouldHandleCookies(false);
        request.setValue_forHTTPHeaderField(Some(&NSString::from_str(cookie)), &NSString::from_str("Cookie"));
        request.setValue_forHTTPHeaderField(Some(&NSString::from_str("application/json")), &NSString::from_str("Accept"));
        let session = NSURLSession::sessionWithConfiguration(&NSURLSessionConfiguration::ephemeralSessionConfiguration());
        let (tx, rx) = mpsc::channel();
        let done = RcBlock::new(move |data: *mut NSData, response: *mut NSURLResponse, error: *mut NSError| {
            // SAFETY: URLSession passes each as nil or an object alive for the call.
            let (data, response, error) = unsafe { (data.as_ref(), response.as_ref(), error.as_ref()) };
            let answer = match error {
                Some(error) => Err(error.localizedDescription().to_string()),
                None => Ok((response.and_then(|r| r.downcast_ref::<NSHTTPURLResponse>()).map_or(0, |r| r.statusCode()), data.map(NSData::to_vec).unwrap_or_default())),
            };
            let _ = tx.send(answer);
        });
        // SAFETY: the block only sends on a channel, which may be done from any thread.
        let task = unsafe { session.dataTaskWithRequest_completionHandler(&request, &done) };
        task.resume();
        let answer = rx.recv_timeout(TIMEOUT + Duration::from_secs(5)).map_err(|_| "claude.ai didn't answer".to_owned()).and_then(|answer| answer);
        session.finishTasksAndInvalidate();
        answer
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_renewing_plan_renews_at_its_next_charge_and_a_cancelled_one_ends() {
        let renewing = serde_json::json!({"status": "active", "plan_ending_at": null, "next_charge_at": "2026-11-22T16:47:25Z"});
        assert_eq!(parse_subscription(&renewing), Some(Subscription { until: 1_795_366_045, renews: Some(true) }));
        let cancelled = serde_json::json!({"status": "active", "plan_ending_at": "2026-11-22T16:47:25Z", "next_charge_at": null});
        assert_eq!(parse_subscription(&cancelled), Some(Subscription { until: 1_795_366_045, renews: Some(false) }));
        assert_eq!(parse_subscription(&serde_json::json!({"status": "active"})), None);
    }

    #[test]
    fn only_organizations_that_can_chat_are_listed() {
        let root = serde_json::json!([
            {"uuid": "api-org", "capabilities": ["api"]},
            {"uuid": "max-org", "capabilities": ["chat", "claude_max"]},
            {"uuid": "", "capabilities": ["chat"]}
        ]);
        assert_eq!(chat_organizations(&root), ["max-org"]);
        assert!(chat_organizations(&serde_json::json!({"error": "x"})).is_empty());
    }
}
