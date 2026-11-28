//! Sleep: what this Mac does when it sits idle, what keeps it awake now, its lid, and when it
//! last slept and woke. Holding it awake is a power assertion, as `caffeinate` makes, which
//! needs no rights. Changing the idle timers, and staying awake with the lid closed, need an
//! administrator: those ask with macOS's own password dialog, and the lid's is undone by a
//! watcher that outlives a crash of this app.
//!
//! The idle timers come from `pmset -g custom`, the one place every power source's settings
//! are listed; the rest is IOKit and two `sysctl`s, a few milliseconds in all.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::iokit::{self, PowerSources};

/// A power source with idle timers of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Battery,
    Adapter,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Battery => "Battery",
            Source::Adapter => "Power adapter",
        }
    }

    /// `pmset`'s flag for the source's settings.
    fn flag(self) -> &'static str {
        match self {
            Source::Battery => "-b",
            Source::Adapter => "-c",
        }
    }
}

/// One source's idle settings, as `pmset` lists them. Minutes are `Some(0)` for never; a
/// setting this Mac doesn't have is `None`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Timers {
    /// Minutes idle before the display turns off.
    pub display: Option<u32>,
    /// Minutes idle before the Mac sleeps.
    pub sleep: Option<u32>,
    pub low_power: Option<bool>,
    /// Wake for network access.
    pub wake_network: Option<bool>,
    pub power_nap: Option<bool>,
}

/// A setting of [`Timers`] that can be changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    Display(u32),
    Sleep(u32),
    LowPower(bool),
    WakeNetwork(bool),
    PowerNap(bool),
}

impl Setting {
    /// The `pmset` arguments that make it so, keeping the Mac's sleep no sooner than its
    /// display's, as System Settings does.
    fn args(self, now: &Timers) -> Vec<String> {
        let flag = |on: bool| if on { "1" } else { "0" }.to_owned();
        let sooner = |a: u32, b: u32| a != 0 && (b == 0 || a < b);
        match self {
            Setting::Display(minutes) => {
                let mut args = vec!["displaysleep".to_owned(), minutes.to_string()];
                // A display that never turns off leaves the Mac's own timer as it is.
                if let Some(sleep) = now.sleep
                    && minutes != 0
                    && sooner(sleep, minutes)
                {
                    args.extend(["sleep".to_owned(), minutes.to_string()]);
                }
                args
            }
            Setting::Sleep(minutes) => {
                let mut args = vec!["sleep".to_owned(), minutes.to_string()];
                if let Some(display) = now.display
                    && sooner(minutes, display)
                {
                    args.extend(["displaysleep".to_owned(), minutes.to_string()]);
                }
                args
            }
            Setting::LowPower(on) => vec!["lowpowermode".to_owned(), flag(on)],
            Setting::WakeNetwork(on) => vec!["womp".to_owned(), flag(on)],
            Setting::PowerNap(on) => vec!["powernap".to_owned(), flag(on)],
        }
    }
}

/// The lid of a notebook.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lid {
    pub closed: bool,
    /// Whether closing it sleeps the Mac: not with sleep turned off, or in clamshell mode.
    pub causes_sleep: bool,
}

/// An app holding sleep off, all its assertions together.
#[derive(Clone, Debug, PartialEq)]
pub struct Holder {
    pub name: String,
    /// Its bundle or executable, for its icon.
    pub path: Option<PathBuf>,
    /// It keeps the display on, not only the Mac.
    pub display: bool,
    /// When its oldest assertion was made, in Unix seconds.
    pub since: Option<i64>,
    /// What its assertions are called ("Playing audio", "Handoff").
    pub reasons: Vec<String>,
}

/// Sleep as macOS has it now.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PowerView {
    /// `None` on a Mac without a battery.
    pub battery: Option<Timers>,
    pub adapter: Timers,
    /// Running on the battery now.
    pub on_battery: bool,
    /// `None` on a Mac without a lid.
    pub lid: Option<Lid>,
    /// `pmset disablesleep`: the Mac sleeps for nothing, not its lid, not the Apple menu.
    pub sleep_disabled: bool,
    pub holders: Vec<Holder>,
    pub slept_at: Option<i64>,
    pub woke_at: Option<i64>,
    /// Why it last woke, in a few words ("lid opened"), when IOKit's reason is a known one.
    pub wake_reason: Option<String>,
    /// Whether `pmset` could be read: without it the timers are unknown.
    pub timers_read: bool,
}

impl PowerView {
    pub fn timers(&self, source: Source) -> Option<&Timers> {
        match source {
            Source::Battery => self.battery.as_ref(),
            Source::Adapter => Some(&self.adapter),
        }
    }

    /// The source the Mac runs on now.
    pub fn source(&self) -> Source {
        if self.on_battery && self.battery.is_some() { Source::Battery } else { Source::Adapter }
    }
}

/// Everything at once: the timers (a run of `pmset`, so off the main thread) and the rest.
pub fn read() -> PowerView {
    let mut view = read_state();
    if let Some((battery, adapter)) = read_timers() {
        view.battery = battery;
        view.adapter = adapter;
        view.timers_read = true;
    }
    view
}

/// What changes from moment to moment, without `pmset`: the lid, who holds sleep off, the
/// last sleep and wake. A few milliseconds.
pub fn read_state() -> PowerView {
    let mut view = PowerView { on_battery: PowerSources::copy().and_then(|sources| sources.providing()).is_some_and(|kind| kind == "Battery Power"), ..PowerView::default() };
    if let Some(root) = iokit::services(c"IOPMrootDomain").next()
        && let Some(properties) = root.properties()
        && let Some(root) = properties.dict()
    {
        view.lid = root.bool("AppleClamshellState").map(|closed| Lid { closed, causes_sleep: root.bool("AppleClamshellCausesSleep").unwrap_or(true) });
        view.sleep_disabled = root.bool("SleepDisabled").unwrap_or(false);
        view.wake_reason = root.string("Wake Reason").as_deref().and_then(wake_words);
    }
    view.slept_at = boot_time(c"kern.sleeptime");
    view.woke_at = boot_time(c"kern.waketime");
    view.holders = holders();
    view
}

/// IOKit's wake reason ("smc.sysState.Wake(0x70070000) lid SMC.OutboxNotEmpty") in words,
/// for the reasons that read plainly; the rest say nothing rather than guess.
fn wake_words(reason: &str) -> Option<String> {
    let reason = reason.to_ascii_lowercase();
    let words = if reason.contains("lid") {
        "lid opened"
    } else if reason.contains("pwrbtn") || reason.contains("power button") {
        "power button"
    } else if reason.contains("rtc") || reason.contains("maint") {
        "scheduled wake"
    } else if reason.contains("usb") || reason.contains("hid") || reason.contains("keyboard") || reason.contains("trackpad") {
        "keyboard or trackpad"
    } else if reason.contains("wlan") || reason.contains("network") || reason.contains("en0") {
        "network"
    } else {
        return None;
    };
    Some(words.to_owned())
}

/// A `sysctl` that holds a moment, as Unix seconds; `None` before it first happened.
fn boot_time(name: &std::ffi::CStr) -> Option<i64> {
    let mut value = libc::timeval { tv_sec: 0, tv_usec: 0 };
    let mut size = std::mem::size_of::<libc::timeval>();
    // SAFETY: `value` has room for the `timeval` these names hold, and `size` says so.
    let ok = unsafe { libc::sysctlbyname(name.as_ptr(), &mut value as *mut libc::timeval as *mut libc::c_void, &mut size, std::ptr::null_mut(), 0) } == 0;
    (ok && value.tv_sec > 0).then_some(value.tv_sec)
}

/// When this Mac started, in Unix seconds.
pub fn booted_at() -> Option<i64> {
    boot_time(c"kern.boottime")
}

/// Reads `pmset -g custom`: the battery's timers (`None` without a battery) and the adapter's.
fn read_timers() -> Option<(Option<Timers>, Timers)> {
    let output = Command::new("/usr/bin/pmset").args(["-g", "custom"]).output().ok()?;
    output.status.success().then(|| parse_custom(&String::from_utf8_lossy(&output.stdout)))
}

/// `pmset -g custom`'s listing: a heading per source, then a setting and its value a line.
fn parse_custom(text: &str) -> (Option<Timers>, Timers) {
    let (mut battery, mut adapter) = (None::<Timers>, Timers::default());
    let mut current: Option<Source> = None;
    for line in text.lines() {
        if !line.starts_with(' ') {
            current = match line.trim_end_matches(':').trim() {
                "Battery Power" => {
                    battery.get_or_insert_with(Timers::default);
                    Some(Source::Battery)
                }
                "AC Power" => Some(Source::Adapter),
                _ => None,
            };
            continue;
        }
        let Some(source) = current else { continue };
        let timers = match source {
            Source::Battery => battery.get_or_insert_with(Timers::default),
            Source::Adapter => &mut adapter,
        };
        // A value is the line's last word; names may have spaces ("Sleep On Power Button").
        let line = line.trim();
        let Some((name, value)) = line.rsplit_once(char::is_whitespace) else { continue };
        let Ok(number) = value.trim().parse::<u32>() else { continue };
        match name.trim() {
            "displaysleep" => timers.display = Some(number),
            "sleep" => timers.sleep = Some(number),
            "lowpowermode" => timers.low_power = Some(number != 0),
            "womp" => timers.wake_network = Some(number != 0),
            "powernap" => timers.power_nap = Some(number != 0),
            _ => {}
        }
    }
    (battery, adapter)
}

/// Assertion types that hold the display on, and those that hold only the Mac awake. The
/// rest (a user being active, a background task) don't stop the Mac sleeping when idle.
const DISPLAY_TYPES: [&str; 2] = ["PreventUserIdleDisplaySleep", "NoDisplaySleepAssertion"];
const SYSTEM_TYPES: [&str; 3] = ["PreventUserIdleSystemSleep", "NoIdleSleepAssertion", "PreventSystemSleep"];

/// The apps holding sleep off, the longest-held first. `powerd`'s own "while the display is
/// on" is the display being on, not something keeping it so, and is left out.
fn holders() -> Vec<Holder> {
    let Some(all) = iokit::assertions_by_process() else { return Vec::new() };
    let Some(all) = all.dict() else { return Vec::new() };
    let mut holders: Vec<(String, Holder)> = Vec::new();
    for (pid, assertions) in all.arrays_by_number() {
        for assertion in assertions {
            let kind = assertion.string("AssertType").unwrap_or_default();
            let display = DISPLAY_TYPES.contains(&kind.as_str());
            if !(display || SYSTEM_TYPES.contains(&kind.as_str())) || assertion.i64("AssertLevel").is_some_and(|level| level == 0) {
                continue;
            }
            let reason = assertion.string("AssertName").unwrap_or_default();
            // `caffeinate` and friends say whom they act for: that app is the one to show.
            let owner = assertion.i64("AssertionOnBehalfOfPID").filter(|&p| p > 0).unwrap_or(pid) as libc::pid_t;
            let (name, path) = process(owner);
            if name == "powerd" {
                continue;
            }
            let since = assertion.date("AssertStartWhen").map(|at| at as i64);
            let key = path.as_ref().map_or_else(|| name.clone(), |p| p.to_string_lossy().into_owned());
            match holders.iter_mut().find(|(k, _)| *k == key) {
                Some((_, holder)) => {
                    holder.display |= display;
                    holder.since = match (holder.since, since) {
                        (Some(a), Some(b)) => Some(a.min(b)),
                        (a, b) => a.or(b),
                    };
                    if !reason.is_empty() && !holder.reasons.contains(&reason) {
                        holder.reasons.push(reason);
                    }
                }
                None => holders.push((key, Holder { name, path, display, since, reasons: if reason.is_empty() { Vec::new() } else { vec![reason] } })),
            }
        }
    }
    let mut holders: Vec<Holder> = holders.into_iter().map(|(_, holder)| holder).collect();
    holders.sort_by_key(|holder| holder.since.unwrap_or(i64::MAX));
    holders
}

/// A process's app: the outermost `.app` its executable is in, named as Finder names it, or
/// for a program outside any app, the program.
fn process(pid: libc::pid_t) -> (String, Option<PathBuf>) {
    let mut buffer = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: the buffer holds the longest path this call writes.
    let len = unsafe { libc::proc_pidpath(pid, buffer.as_mut_ptr() as *mut libc::c_void, buffer.len() as u32) };
    if len <= 0 {
        return (format!("Process {pid}"), None);
    }
    let path = PathBuf::from(String::from_utf8_lossy(&buffer[..len as usize]).into_owned());
    let mut bundle = PathBuf::new();
    for part in path.components() {
        bundle.push(part);
        if part.as_os_str().to_str().is_some_and(|name| name.ends_with(".app")) {
            let name = bundle.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
            return (name, Some(bundle));
        }
    }
    let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    (name, Some(path))
}

/// An assertion this app holds; macOS lets it go when it is dropped, when its timeout passes,
/// or when the app quits.
#[derive(Debug)]
pub struct Hold(u32);

impl Drop for Hold {
    fn drop(&mut self) {
        iokit::release_awake(self.0);
    }
}

/// Holds the Mac awake (and its display on, with `display`) for `seconds`, or until let go.
pub fn hold(display: bool, seconds: Option<i64>) -> Option<Hold> {
    let kind = if display { "PreventUserIdleDisplaySleep" } else { "PreventUserIdleSystemSleep" };
    let reason = if display { "Keeping the display on, as asked in Nerda Bar" } else { "Keeping this Mac awake, as asked in Nerda Bar" };
    iokit::assert_awake(kind, "Nerda Bar keep awake", reason, seconds.unwrap_or(0) as f64).map(Hold)
}

/// Runs `script` as root, after macOS's password dialog says `prompt`. `Err` says why not,
/// "Cancelled" when the dialog was.
fn as_admin(script: &str, prompt: &str) -> Result<(), String> {
    let output = Command::new("/usr/bin/osascript").args(["-e", &apple_script(script, prompt)]).output().map_err(|e| e.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    let error = String::from_utf8_lossy(&output.stderr);
    if error.contains("-128") { Err("Cancelled".to_owned()) } else { Err(error.trim().rsplit(": ").next().unwrap_or("It didn't work").to_owned()) }
}

/// The AppleScript that runs `script` as root after a password dialog saying `prompt`.
fn apple_script(script: &str, prompt: &str) -> String {
    let quote = |text: &str| text.replace('\\', "\\\\").replace('"', "\\\"");
    format!("do shell script \"{}\" with prompt \"{}\" with administrator privileges", quote(script), quote(prompt))
}

/// Changes one of `source`'s settings, `now` being what it has.
pub fn set(source: Source, setting: Setting, now: &Timers) -> Result<(), String> {
    let args = setting.args(now).join(" ");
    as_admin(&format!("/usr/bin/pmset {} {args}", source.flag()), "Nerda Bar wants to change when this Mac sleeps.")
}

/// Turns sleep off so that closing the lid doesn't sleep the Mac, and starts a watcher, as
/// root, that turns it back on as soon as `flag` is gone or this app (`pid`) has quit or
/// crashed. Removing `flag` needs no password.
pub fn hold_lid(flag: &Path, pid: u32) -> Result<(), String> {
    // Sleep was already off before this, by the person's own `pmset`: there is nothing to
    // turn off, and nothing of ours to turn back on afterwards.
    if read_state().sleep_disabled && !flag.exists() {
        return Ok(());
    }
    let flag = flag.to_str().filter(|path| !path.contains(['\'', '"', '\\', '$', '`'])).ok_or("The data folder's name can't be used in a command.")?;
    std::fs::write(flag, format!("{pid}\n")).map_err(|e| e.to_string())?;
    let result = as_admin(&lid_watcher(flag, pid, "/usr/bin/pmset -a disablesleep"), "Nerda Bar wants to keep this Mac awake with its lid closed.");
    if result.is_err() {
        let _ = std::fs::remove_file(flag);
    }
    result
}

/// The command that turns sleep off with `switch` ("pmset -a disablesleep"), then waits in the
/// background while `flag` is there and `pid` runs, and turns it back on. It turns it back on
/// too when it is itself told to end, as every process is when the Mac shuts down or restarts:
/// without that, sleep would stay off after the restart. It waits on its `sleep` rather than
/// in it, so the signal is acted on at once.
fn lid_watcher(flag: &str, pid: u32, switch: &str) -> String {
    let watch = format!(
        "restore() {{ {switch} 0; /bin/rm -f \"{flag}\"; }}; trap \"restore; exit 0\" TERM INT HUP; {switch} 1; while /bin/kill -0 {pid} 2>/dev/null && [ -e \"{flag}\" ]; do /bin/sleep 2 & wait $!; done; restore"
    );
    format!("/usr/bin/nohup /bin/sh -c '{watch}' >/dev/null 2>&1 &")
}

/// Ends [`hold_lid`]: its watcher turns sleep back on within two seconds.
pub fn release_lid(flag: &Path) {
    let _ = std::fs::remove_file(flag);
}

/// Turns sleep back on, when an earlier run's watcher couldn't (the Mac was shut down with it).
pub fn allow_sleep(flag: &Path) -> Result<(), String> {
    as_admin("/usr/bin/pmset -a disablesleep 0", "Nerda Bar wants to let this Mac sleep again.")?;
    let _ = std::fs::remove_file(flag);
    Ok(())
}

/// Turns the display off now; the Mac stays awake.
pub fn display_off() {
    let _ = Command::new("/usr/bin/pmset").arg("displaysleepnow").status();
}

/// Puts the Mac to sleep now.
pub fn sleep_now() {
    let _ = Command::new("/usr/bin/pmset").arg("sleepnow").status();
}

/// Locks the screen, as Control-Command-Q does.
pub fn lock_screen() -> Result<(), String> {
    type Lock = unsafe extern "C" fn() -> i32;
    // SAFETY: loads a system framework and looks a symbol up by name; it is a function taking
    // nothing when present. It is private to macOS, so its absence is allowed for.
    unsafe {
        let framework = libc::dlopen(c"/System/Library/PrivateFrameworks/login.framework/Versions/Current/login".as_ptr(), libc::RTLD_LAZY);
        if framework.is_null() {
            return Err("This macOS can't lock from here.".to_owned());
        }
        let symbol = libc::dlsym(framework, c"SACLockScreenImmediate".as_ptr());
        if symbol.is_null() {
            return Err("This macOS can't lock from here.".to_owned());
        }
        std::mem::transmute::<*mut libc::c_void, Lock>(symbol)();
    }
    Ok(())
}

/// A Mac part-way through a day, for headless renders: a notebook on battery, three apps
/// holding sleep off.
pub fn demo(now: i64) -> PowerView {
    let holder = |name: &str, path: &str, display: bool, ago: i64, reasons: &[&str]| Holder {
        name: name.to_owned(),
        path: Some(PathBuf::from(path)),
        display,
        since: Some(now - ago),
        reasons: reasons.iter().map(|r| (*r).to_owned()).collect(),
    };
    PowerView {
        battery: Some(Timers { display: Some(5), sleep: Some(15), low_power: Some(false), wake_network: Some(false), power_nap: Some(false) }),
        adapter: Timers { display: Some(20), sleep: Some(0), low_power: Some(false), wake_network: Some(true), power_nap: Some(true) },
        on_battery: true,
        lid: Some(Lid { closed: false, causes_sleep: true }),
        sleep_disabled: false,
        holders: vec![
            holder("Terminal", "/System/Applications/Utilities/Terminal.app", false, 18 * 3600 + 20 * 60, &["caffeinate command-line tool"]),
            holder("Claude", "/Applications/Claude.app", false, 3 * 3600 + 39 * 60, &["Electron"]),
            holder("Music", "/System/Applications/Music.app", false, 42 * 60, &["Playing audio"]),
            holder("sharingd", "/usr/libexec/sharingd", false, 164, &["Handoff"]),
        ],
        slept_at: Some(now - 3 * 3600 - 51 * 60),
        woke_at: Some(now - 3 * 3600 - 45 * 60),
        wake_reason: Some("lid opened".to_owned()),
        timers_read: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CUSTOM: &str = "Battery Power:\n Sleep On Power Button 1\n lowpowermode         0\n powernap             0\n displaysleep         5\n womp                 0\n sleep                15\nAC Power:\n lowpowermode         0\n powernap             1\n displaysleep         20\n womp                 1\n sleep                0\n";

    #[test]
    fn pmset_lists_each_sources_timers() {
        let (battery, adapter) = parse_custom(CUSTOM);
        assert_eq!(battery, Some(Timers { display: Some(5), sleep: Some(15), low_power: Some(false), wake_network: Some(false), power_nap: Some(false) }));
        assert_eq!(adapter, Timers { display: Some(20), sleep: Some(0), low_power: Some(false), wake_network: Some(true), power_nap: Some(true) });
        let (battery, adapter) = parse_custom("AC Power:\n displaysleep 10\n sleep 0\n");
        assert_eq!((battery, adapter.display), (None, Some(10)), "a Mac without a battery lists only the adapter");
    }

    #[test]
    fn a_wake_reads_in_words_or_not_at_all() {
        assert_eq!(wake_words("smc.sysState.Wake(0x70070000) lid SMC.OutboxNotEmpty").as_deref(), Some("lid opened"));
        assert_eq!(wake_words("EC.SleepTimer (SleepTimer)"), None);
    }

    #[test]
    fn the_mac_never_sleeps_before_its_display() {
        let now = Timers { display: Some(10), sleep: Some(15), ..Timers::default() };
        assert_eq!(Setting::Display(30).args(&now), ["displaysleep", "30", "sleep", "30"]);
        assert_eq!(Setting::Display(5).args(&now), ["displaysleep", "5"]);
        assert_eq!(Setting::Sleep(5).args(&now), ["sleep", "5", "displaysleep", "5"]);
        assert_eq!(Setting::Sleep(0).args(&now), ["sleep", "0"]);
        assert_eq!(Setting::Display(0).args(&now), ["displaysleep", "0"], "a display that never turns off leaves the Mac's timer alone");
        let never = Timers { display: Some(10), sleep: Some(0), ..Timers::default() };
        assert_eq!(Setting::Display(60).args(&never), ["displaysleep", "60"]);
    }

    /// The scripts compile as AppleScript, and the lid's watcher, run as this user with a
    /// marker file standing in for `pmset`, returns at once, then turns "sleep" back on when
    /// its flag goes and when the app it watches ends. Nothing here needs a password or
    /// changes the Mac. Run with `--ignored`.
    #[test]
    #[ignore]
    fn the_privileged_scripts_compile_and_the_watcher_lets_go() {
        let dir = std::env::temp_dir().join(format!("nerda-bar-watch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (flag, marker, compiled) = (dir.join("flag"), dir.join("marker"), dir.join("check.scpt"));
        let switch = format!("/bin/sh -c \"echo \\$0 >> {}\"", marker.display());
        for script in [lid_watcher(flag.to_str().unwrap(), 1, "/usr/bin/pmset -a disablesleep"), "/usr/bin/pmset -b displaysleep 10".to_owned()] {
            let apple = apple_script(&script, "Nerda Bar wants to check.");
            let status = Command::new("/usr/bin/osacompile").args(["-e", &apple, "-o"]).arg(&compiled).status().unwrap();
            assert!(status.success(), "{apple} compiles");
        }
        let wait_for = |want: &str| {
            for _ in 0..50 {
                if std::fs::read_to_string(&marker).unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ") == want {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            panic!("the marker reads {:?}, not {want:?}", std::fs::read_to_string(&marker).unwrap_or_default());
        };
        // Watching this test, until the flag goes.
        std::fs::write(&flag, "").unwrap();
        let started = std::time::Instant::now();
        assert!(Command::new("/bin/sh").args(["-c", &lid_watcher(flag.to_str().unwrap(), std::process::id(), &switch)]).status().unwrap().success());
        assert!(started.elapsed().as_secs_f32() < 1.0, "the watcher runs in the background");
        wait_for("1");
        std::fs::remove_file(&flag).unwrap();
        wait_for("1 0");
        // Watching a process that ends, with the flag still there.
        std::fs::write(&marker, "").unwrap();
        std::fs::write(&flag, "").unwrap();
        let mut child = Command::new("/bin/sleep").arg("1").spawn().unwrap();
        assert!(Command::new("/bin/sh").args(["-c", &lid_watcher(flag.to_str().unwrap(), child.id(), &switch)]).status().unwrap().success());
        child.wait().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2500));
        wait_for("1 0");
        assert!(!flag.exists(), "the watcher takes its flag with it");
        // Told to end, as at shutdown, with its app still running and its flag still there.
        std::fs::write(&marker, "").unwrap();
        std::fs::write(&flag, "").unwrap();
        assert!(Command::new("/bin/sh").args(["-c", &lid_watcher(flag.to_str().unwrap(), std::process::id(), &switch)]).status().unwrap().success());
        wait_for("1");
        let watchers = Command::new("/usr/bin/pgrep").args(["-f", &format!("restore.*{}", flag.display())]).output().unwrap();
        for pid in String::from_utf8_lossy(&watchers.stdout).split_whitespace() {
            let _ = Command::new("/bin/kill").args(["-TERM", pid]).status();
        }
        wait_for("1 0");
        assert!(!flag.exists(), "a watcher told to end turns sleep back on first");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The lock is found on this macOS (it is not called).
    #[test]
    #[ignore]
    fn the_lock_is_there() {
        // SAFETY: loads a system framework and looks a symbol up; nothing is called.
        let found = unsafe {
            let framework = libc::dlopen(c"/System/Library/PrivateFrameworks/login.framework/Versions/Current/login".as_ptr(), libc::RTLD_LAZY);
            !framework.is_null() && !libc::dlsym(framework, c"SACLockScreenImmediate".as_ptr()).is_null()
        };
        assert!(found);
    }

    /// Reads this Mac: the apps holding sleep off and the lid. Run with `--ignored --nocapture`.
    #[test]
    #[ignore]
    fn reads_this_mac() {
        let view = read();
        eprintln!("{view:#?}");
        let hold = hold(true, Some(30)).expect("an assertion");
        assert!(read_state().holders.iter().any(|h| h.reasons.iter().any(|r| r == "Nerda Bar keep awake")));
        drop(hold);
    }
}
