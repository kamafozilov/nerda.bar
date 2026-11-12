//! What each app uses: every process this user can read, summed by the app it belongs to, so a
//! browser's dozens of helpers and renderers read as the one browser.
//!
//! A process belongs to the outermost `.app` bundle its executable is in. One outside any app
//! (a WebKit page, which the system runs from its own framework on an app's behalf) belongs to
//! the app macOS holds responsible for it; anything else (a command line tool) stands alone
//! under its own name. Memory is the process's footprint, the figure Activity Monitor shows;
//! CPU is the time it ran since the last reading, as a share of every core.

// libc marks the Mach timebase deprecated in favour of the `mach2` crate; one call doesn't
// warrant another dependency.
#![allow(deprecated)]

use std::collections::HashMap;
use std::ffi::CStr;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// One app, all its processes together.
#[derive(Clone, Debug, PartialEq)]
pub struct AppUsage {
    pub name: String,
    /// The bundle or executable, for its icon.
    pub path: Option<PathBuf>,
    /// Bytes.
    pub memory: u64,
    /// A share of all the cores, 0 to 1.
    pub cpu: f32,
    pub processes: usize,
}

/// Which app a process belongs to.
#[derive(Clone, Debug)]
struct Owner {
    key: String,
    name: String,
    path: Option<PathBuf>,
}

type Responsible = unsafe extern "C" fn(libc::pid_t) -> libc::pid_t;

pub struct Apps {
    /// Mach time units to nanoseconds.
    timebase: (u64, u64),
    /// A process, known by its pid and start time (pids are reused), and its owner.
    owners: HashMap<(libc::pid_t, u64), Owner>,
    /// Each process's CPU time at the last reading, in nanoseconds.
    last: HashMap<(libc::pid_t, u64), u64>,
    last_at: Option<Instant>,
    responsible: Option<Responsible>,
}

impl Apps {
    pub fn new() -> Apps {
        let mut info = libc::mach_timebase_info { numer: 1, denom: 1 };
        // SAFETY: fills the struct it is given.
        unsafe { libc::mach_timebase_info(&mut info) };
        // SAFETY: looks a symbol up by name; a function of this signature when present. It is
        // private to macOS, so its absence is allowed for.
        let responsible = unsafe {
            let symbol = libc::dlsym(libc::RTLD_DEFAULT, c"responsibility_get_pid_responsible_for_pid".as_ptr());
            (!symbol.is_null()).then(|| std::mem::transmute::<*mut libc::c_void, Responsible>(symbol))
        };
        Apps { timebase: (info.numer.max(1) as u64, info.denom.max(1) as u64), owners: HashMap::new(), last: HashMap::new(), last_at: None, responsible }
    }

    /// Whether the last reading is recent enough for the next one to measure CPU against.
    pub fn primed(&self) -> bool {
        self.last_at.is_some_and(|at| at.elapsed().as_secs_f64() < 5.0)
    }

    /// Every app with a process this user can read, in no order.
    pub fn read(&mut self, cores: usize) -> Vec<AppUsage> {
        let now = Instant::now();
        let elapsed = self.last_at.map(|at| now.duration_since(at).as_nanos() as f64).filter(|_| self.primed());
        let mut apps: HashMap<String, AppUsage> = HashMap::new();
        let mut seen = HashMap::with_capacity(self.last.len());
        for pid in all_pids() {
            let Some(usage) = rusage(pid) else { continue };
            let id = (pid, usage.ri_proc_start_abstime);
            let cpu_time = (usage.ri_user_time + usage.ri_system_time) as u128 * self.timebase.0 as u128 / self.timebase.1 as u128;
            let cpu_time = cpu_time as u64;
            let ran = match (elapsed, self.last.get(&id)) {
                (Some(elapsed), Some(&before)) if elapsed > 0.0 => (cpu_time.saturating_sub(before) as f64 / (elapsed * cores.max(1) as f64)) as f32,
                _ => 0.0,
            };
            seen.insert(id, cpu_time);
            let responsible = self.responsible;
            let owner = self.owners.entry(id).or_insert_with(|| owner_of(pid, responsible));
            let app = apps.entry(owner.key.clone()).or_insert_with(|| AppUsage { name: owner.name.clone(), path: owner.path.clone(), memory: 0, cpu: 0.0, processes: 0 });
            app.memory += usage.ri_phys_footprint;
            app.cpu += ran;
            app.processes += 1;
        }
        self.owners.retain(|id, _| seen.contains_key(id));
        self.last = seen;
        self.last_at = Some(now);
        apps.into_values().collect()
    }
}

fn all_pids() -> Vec<libc::pid_t> {
    // SAFETY: the first call returns how many there are; the second fills at most the buffer.
    unsafe {
        let count = libc::proc_listallpids(std::ptr::null_mut(), 0);
        if count <= 0 {
            return Vec::new();
        }
        // Room for processes started between the two calls.
        let mut pids = vec![0 as libc::pid_t; count as usize + 64];
        let n = libc::proc_listallpids(pids.as_mut_ptr() as *mut libc::c_void, (pids.len() * size_of::<libc::pid_t>()) as libc::c_int);
        pids.truncate(n.max(0) as usize);
        pids.retain(|&pid| pid > 0);
        pids
    }
}

fn rusage(pid: libc::pid_t) -> Option<libc::rusage_info_v4> {
    // SAFETY: the kernel fills a `rusage_info_v4` for this flavor.
    unsafe {
        let mut info: libc::rusage_info_v4 = std::mem::zeroed();
        let ok = libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V4, &mut info as *mut _ as *mut libc::rusage_info_t) == 0;
        ok.then_some(info)
    }
}

fn executable(pid: libc::pid_t) -> Option<PathBuf> {
    let mut buffer = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: writes at most the buffer's length.
    let n = unsafe { libc::proc_pidpath(pid, buffer.as_mut_ptr() as *mut libc::c_void, buffer.len() as u32) };
    (n > 0).then(|| {
        buffer.truncate(n as usize);
        PathBuf::from(String::from_utf8_lossy(&buffer).into_owned())
    })
}

fn process_name(pid: libc::pid_t) -> Option<String> {
    let mut buffer = [0u8; 256];
    // SAFETY: writes at most the buffer's length, nul-terminated.
    let n = unsafe { libc::proc_name(pid, buffer.as_mut_ptr() as *mut libc::c_void, buffer.len() as u32) };
    (n > 0).then(|| CStr::from_bytes_until_nul(&buffer).ok().map(|s| s.to_string_lossy().into_owned())).flatten()
}

/// The outermost app bundle on `path`: `/Applications/Helium.app` for its helpers deep inside.
fn outermost_app(path: &Path) -> Option<PathBuf> {
    let mut app = PathBuf::new();
    for part in path.components() {
        app.push(part);
        if part.as_os_str().to_string_lossy().ends_with(".app") {
            return Some(app);
        }
    }
    None
}

/// The name Finder shows for a bundle: "Visual Studio Code" for `Visual Studio Code.app`.
fn bundle_name(bundle: &Path) -> String {
    let stem = bundle.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    #[cfg(target_os = "macos")]
    {
        use objc2_foundation::{NSFileManager, NSString};
        let shown = NSFileManager::defaultManager().displayNameAtPath(&NSString::from_str(&bundle.to_string_lossy())).to_string();
        let shown = shown.strip_suffix(".app").unwrap_or(&shown).trim().to_owned();
        if !shown.is_empty() {
            return shown;
        }
    }
    stem
}

fn owner_of(pid: libc::pid_t, responsible: Option<Responsible>) -> Owner {
    let path = executable(pid);
    let app = |bundle: PathBuf| Owner { key: bundle.to_string_lossy().into_owned(), name: bundle_name(&bundle), path: Some(bundle) };
    if let Some(bundle) = path.as_deref().and_then(outermost_app) {
        return app(bundle);
    }
    // Work the system runs for an app from outside it (a WebKit page, an extension).
    let on_behalf = path.as_deref().is_some_and(|p| {
        let p = p.to_string_lossy();
        p.contains(".xpc/") || p.contains(".appex/")
    });
    if on_behalf
        && let Some(responsible) = responsible
    {
        // SAFETY: takes a pid, returns a pid or -1.
        let parent = unsafe { responsible(pid) };
        if parent > 0
            && parent != pid
            && let Some(bundle) = executable(parent).as_deref().and_then(outermost_app)
        {
            return app(bundle);
        }
    }
    let name = path.as_deref().and_then(tool_name).or_else(|| process_name(pid)).unwrap_or_else(|| format!("Process {pid}"));
    Owner { key: format!("process:{name}"), name, path }
}

/// A command line tool's name: its file's, or for a binary named by its version (Claude Code
/// installs `claude/versions/2.1.322`), the folder it is kept under.
fn tool_name(path: &Path) -> Option<String> {
    let versioned = |name: &str| name.starts_with(|c: char| c.is_ascii_digit()) || name == "versions";
    path.iter().rev().map(|part| part.to_string_lossy()).find(|part| !versioned(part)).map(|part| part.into_owned())
}

/// The top `count` of `apps` by `measure`, then what the rest add up to and how many there are.
pub fn top(apps: &[AppUsage], count: usize, measure: impl Fn(&AppUsage) -> f64) -> (Vec<&AppUsage>, f64, usize) {
    let mut sorted: Vec<&AppUsage> = apps.iter().collect();
    sorted.sort_by(|a, b| measure(b).total_cmp(&measure(a)).then_with(|| a.name.cmp(&b.name)));
    let rest = sorted.split_off(count.min(sorted.len()));
    (sorted, rest.iter().map(|app| measure(app)).sum(), rest.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_helper_belongs_to_the_outermost_app() {
        let helper = Path::new("/Applications/Helium.app/Contents/Frameworks/Helium Framework.framework/Helpers/Helium Helper (Renderer).app/Contents/MacOS/Helium Helper (Renderer)");
        assert_eq!(outermost_app(helper), Some(PathBuf::from("/Applications/Helium.app")));
        assert_eq!(outermost_app(Path::new("/usr/bin/node")), None);
    }

    #[test]
    fn a_tool_named_by_its_version_takes_its_folders_name() {
        assert_eq!(tool_name(Path::new("/Users/me/.local/share/claude/versions/2.1.322")).as_deref(), Some("claude"));
        assert_eq!(tool_name(Path::new("/opt/homebrew/bin/node")).as_deref(), Some("node"));
    }

    #[test]
    fn this_users_apps_read_with_their_processes_summed() {
        let mut apps = Apps::new();
        let first = apps.read(1);
        assert!(!first.is_empty());
        assert!(first.iter().all(|app| app.cpu == 0.0), "no CPU without an earlier reading");
        let _ = std::hint::black_box((0..3_000_000u64).fold(0u64, |a, b| a.wrapping_add(b * b)));
        let second = apps.read(1);
        let total: u64 = second.iter().map(|app| app.memory).sum();
        assert!(total > 0);
        // This test runs inside one: it ran a moment ago, so it has CPU time.
        assert!(second.iter().any(|app| app.cpu > 0.0));
    }

    #[test]
    fn the_top_apps_come_first_and_the_rest_add_up() {
        let app = |name: &str, memory: u64| AppUsage { name: name.to_owned(), path: None, memory, cpu: 0.0, processes: 1 };
        let apps = [app("a", 5), app("b", 30), app("c", 10), app("d", 1)];
        let (top, rest, others) = top(&apps, 2, |a| a.memory as f64);
        assert_eq!(top.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["b", "c"]);
        assert_eq!((rest, others), (6.0, 2));
    }
}
