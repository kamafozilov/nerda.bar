//! This Mac's own measures: the CPU and memory in use, kept over time for the panel's charts
//! and read on their own thread at the rate what shows them needs.
//!
//! The rate follows what is on screen: every second while the panel shows one of these tabs,
//! every 2 seconds while the menu bar shows one, every 20 seconds while only the panel's
//! history needs them, and not at all when nothing does. A reading is a few system calls; what
//! each app uses is read only while the panel shows it, a few milliseconds a second.

pub mod apps;
pub mod history;
mod mac;

use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::Duration;

use apps::{AppUsage, Apps};
use history::{Point, Series};

/// How often the measures are read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rate {
    Off,
    /// Only the panel's history needs them.
    Background,
    /// The menu bar shows one.
    MenuBar,
    /// The panel shows one.
    Live,
}

impl Rate {
    fn seconds(self) -> Option<f64> {
        match self {
            Rate::Off => None,
            Rate::Background => Some(20.0),
            Rate::MenuBar => Some(2.0),
            Rate::Live => Some(1.0),
        }
    }
}

/// What this Mac is, read once.
#[derive(Clone, Debug, PartialEq)]
pub struct Machine {
    /// "M1 Pro".
    pub chip: String,
    pub performance_cores: usize,
    /// Apple silicon numbers its efficiency cores first.
    pub efficiency_cores: usize,
    /// Bytes of memory.
    pub memory: u64,
    pub booted_at: Option<i64>,
}

impl Machine {
    fn read() -> Machine {
        let chip = mac::sysctl_string(c"machdep.cpu.brand_string").unwrap_or_default();
        let chip = chip.strip_prefix("Apple ").unwrap_or(&chip).to_owned();
        let count = |name: &std::ffi::CStr| mac::sysctl::<i32>(name).filter(|&n| n > 0).map(|n| n as usize);
        let (performance_cores, efficiency_cores) = match (count(c"hw.perflevel0.logicalcpu"), count(c"hw.perflevel1.logicalcpu")) {
            (Some(p), e) => (p, e.unwrap_or(0)),
            (None, _) => (count(c"hw.logicalcpu").unwrap_or(1), 0),
        };
        Machine { chip, performance_cores, efficiency_cores, memory: mac::sysctl::<u64>(c"hw.memsize").unwrap_or(0), booted_at: mac::boot_time() }
    }
}

/// The CPU over the time since the last reading, as shares from 0 to 1.
#[derive(Clone, Debug, PartialEq)]
pub struct Cpu {
    /// Apps, with low-priority ("nice") work.
    pub user: f32,
    pub system: f32,
    /// Each core's share in use, efficiency cores first.
    pub cores: Vec<f32>,
    pub load: [f64; 3],
}

impl Cpu {
    pub fn total(&self) -> f32 {
        (self.user + self.system).clamp(0.0, 1.0)
    }

    fn between(before: &[[u32; 4]], after: &[[u32; 4]], load: [f64; 3]) -> Cpu {
        let (mut user, mut system, mut all) = (0u64, 0u64, 0u64);
        let cores = before
            .iter()
            .zip(after)
            .map(|(a, b)| {
                // The counters are 32 bits and wrap.
                let d: Vec<u64> = (0..4).map(|i| b[i].wrapping_sub(a[i]) as u64).collect();
                let (u, s, idle, n) = (d[0], d[1], d[2], d[3]);
                let total = u + s + idle + n;
                user += u + n;
                system += s;
                all += total;
                if total == 0 { 0.0 } else { (u + s + n) as f32 / total as f32 }
            })
            .collect();
        let share = |part: u64| if all == 0 { 0.0 } else { part as f32 / all as f32 };
        Cpu { user: share(user), system: share(system), cores, load }
    }
}

/// How hard macOS is working to find memory, as Activity Monitor's pressure graph colours it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pressure {
    Normal,
    Warning,
    Critical,
}

/// Memory in bytes, split as Activity Monitor splits it.
#[derive(Clone, Debug, PartialEq)]
pub struct Memory {
    pub total: u64,
    /// Apps' own memory.
    pub app: u64,
    /// Kept in memory by the system, never compressed or swapped.
    pub wired: u64,
    /// Held compressed.
    pub compressed: u64,
    /// Files kept in memory to read faster, given up whenever apps need it.
    pub cached: u64,
    pub swap_used: u64,
    pub pressure: Pressure,
}

impl Memory {
    pub fn used(&self) -> u64 {
        self.app + self.wired + self.compressed
    }

    pub fn used_share(&self) -> f32 {
        if self.total == 0 { 0.0 } else { (self.used() as f64 / self.total as f64).clamp(0.0, 1.0) as f32 }
    }

    fn read(host: &mac::Host, total: u64) -> Option<Memory> {
        let stats = host.vm_statistics()?;
        let page = mac::sysctl::<u64>(c"hw.pagesize").or_else(|| mac::sysctl::<i32>(c"hw.pagesize").map(|p| p as u64)).unwrap_or(16_384);
        let pages = |count: u32| count as u64 * page;
        let purgeable = stats.purgeable_count;
        let swap = mac::sysctl::<libc::xsw_usage>(c"vm.swapusage");
        let pressure = match mac::sysctl::<i32>(c"kern.memorystatus_vm_pressure_level") {
            Some(4) => Pressure::Critical,
            Some(2) => Pressure::Warning,
            _ => Pressure::Normal,
        };
        Some(Memory {
            total,
            app: pages(stats.internal_page_count.saturating_sub(purgeable)),
            wired: pages(stats.wire_count),
            compressed: pages(stats.compressor_page_count),
            cached: pages(stats.external_page_count + purgeable),
            swap_used: swap.map_or(0, |s| s.xsu_used),
            pressure,
        })
    }
}

/// One reading and the history up to it, as the panel and the menu bar read them.
#[derive(Clone, Debug)]
pub struct SystemView {
    /// Unix seconds of the reading.
    pub at: f64,
    pub machine: Arc<Machine>,
    pub cpu: Option<Cpu>,
    pub memory: Option<Memory>,
    /// Per tier of [`history::TIERS`]: the CPU in use (user and system together), its system
    /// part, and the share of memory used.
    pub cpu_total: [Vec<Option<Point>>; 3],
    pub cpu_system: [Vec<Option<Point>>; 3],
    pub memory_used: [Vec<Option<Point>>; 3],
    /// What each app uses, read only while the panel shows this Mac's tabs.
    pub apps: Option<Arc<Vec<AppUsage>>>,
}

impl SystemView {
    /// The last `count` readings of `series` at the finest tier, for a menu bar graph.
    pub fn recent(series: &[Vec<Option<Point>>; 3], count: usize) -> Vec<f32> {
        let fine = &series[0];
        // The bucket holding now is still filling: leave it out until it closes.
        let end = fine.len().saturating_sub(1);
        fine[end.saturating_sub(count)..end].iter().map(|p| p.map_or(0.0, |p| p.avg)).collect()
    }
}

pub struct Sampler {
    rates: Sender<Rate>,
}

impl Sampler {
    pub fn set_rate(&self, rate: Rate) {
        let _ = self.rates.send(rate);
    }
}

fn unix_now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

pub fn spawn(on_view: impl Fn(Arc<SystemView>) + Send + 'static) -> Sampler {
    let (rates, inbox) = mpsc::channel::<Rate>();
    let run = move || {
        let host = mac::Host::new();
        let machine = Arc::new(Machine::read());
        let mut apps = Apps::new();
        let cores = machine.performance_cores + machine.efficiency_cores;
        let mut rate = Rate::Off;
        let (mut cpu_total, mut cpu_system, mut memory_used) = (Series::default(), Series::default(), Series::default());
        let mut last: Option<(f64, Vec<[u32; 4]>)> = None;
        let mut next = f64::INFINITY;
        loop {
            let now = unix_now();
            let message = match rate.seconds() {
                None => inbox.recv().map_err(|_| RecvTimeoutError::Disconnected),
                Some(_) => inbox.recv_timeout(Duration::from_secs_f64((next - now).max(0.0))),
            };
            match message {
                Ok(new) => {
                    let faster = new > rate;
                    rate = new;
                    match rate.seconds() {
                        None => continue,
                        // A faster rate reads at once; a slower one waits out its own step.
                        Some(step) if !faster => {
                            next = last.as_ref().map_or(now, |(at, _)| at + step);
                            continue;
                        }
                        Some(_) => {}
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            let Some(step) = rate.seconds() else { continue };

            // The first reading has nothing to compare with: take a second one a moment later.
            if last.is_none()
                && let Some(ticks) = host.cpu_ticks()
            {
                last = Some((unix_now(), ticks));
                std::thread::sleep(Duration::from_millis(250));
            }
            let now = unix_now();
            let ticks = host.cpu_ticks();
            let load = mac::load_average();
            let cpu = match (&last, &ticks) {
                (Some((_, before)), Some(after)) => Some(Cpu::between(before, after, load)),
                _ => None,
            };
            let memory = Memory::read(&host, machine.memory);
            let app_usage = (rate == Rate::Live).then(|| {
                // CPU time needs an earlier reading to measure against.
                if !apps.primed() {
                    apps.read(cores);
                    std::thread::sleep(Duration::from_millis(300));
                }
                Arc::new(apps.read(cores))
            });
            // A reading stands for the time since the last one, unless the Mac slept or nothing
            // read for long between them: then only for one step.
            let since = last.as_ref().map_or(now - step, |(at, _)| *at);
            let from = if now - since > step * 3.0 + 30.0 { now - step } else { since };
            if let Some(cpu) = &cpu {
                cpu_total.add(from, now, cpu.total());
                cpu_system.add(from, now, cpu.system);
            }
            if let Some(memory) = &memory {
                memory_used.add(from, now, memory.used_share());
            }
            if let Some(ticks) = ticks {
                last = Some((now, ticks));
            }
            let tiers = |series: &Series| [0, 1, 2].map(|tier| series.points(tier, now));
            on_view(Arc::new(SystemView {
                at: now,
                machine: machine.clone(),
                cpu,
                memory,
                cpu_total: tiers(&cpu_total),
                cpu_system: tiers(&cpu_system),
                memory_used: tiers(&memory_used),
                apps: app_usage,
            }));
            next = now + step;
        }
    };
    std::thread::Builder::new().name("nerda-bar-system".to_owned()).spawn(run).expect("the system thread starts");
    Sampler { rates }
}

/// A made-up reading and history for headless renders: a working afternoon on an M1 Pro.
pub fn demo(now: f64) -> SystemView {
    let machine = Arc::new(Machine { chip: "M1 Pro".to_owned(), performance_cores: 8, efficiency_cores: 2, memory: 16 << 30, booted_at: Some(now as i64 - 37 * 86_400 - 5 * 3600) });
    let (mut cpu_total, mut cpu_system, mut memory_used) = (Series::default(), Series::default(), Series::default());
    let wave = |t: f64, seed: f64| ((t * 0.0021 + seed).sin() * 0.5 + (t * 0.013 + seed * 3.0).sin() * 0.3 + (t * 0.071).sin() * 0.2) * 0.5 + 0.5;
    let start = now - 86_400.0;
    let mut t = start;
    while t < now {
        // Coarse far back, fine near now, as a day of real readings would be.
        let step = if now - t > 3600.0 { 60.0 } else { 2.0 };
        let busy = wave(t, 0.4).powf(2.2) as f32;
        let spike = if ((t / 2.0) as i64 % 97) < 3 { 0.35 } else { 0.0 };
        let total = (0.04 + busy * 0.55 + spike).min(0.98);
        cpu_total.add(t, t + step, total);
        cpu_system.add(t, t + step, total * 0.32);
        memory_used.add(t, t + step, (0.71 + wave(t, 2.0) as f32 * 0.12).min(0.97));
        t += step;
    }
    let tiers = |series: &Series| [0, 1, 2].map(|tier| series.points(tier, now));
    SystemView {
        at: now,
        cpu: Some(Cpu {
            user: 0.21,
            system: 0.09,
            cores: vec![0.62, 0.48, 0.71, 0.44, 0.38, 0.22, 0.17, 0.09, 0.05, 0.03],
            load: [3.42, 2.87, 2.51],
        }),
        memory: Some(Memory {
            total: machine.memory,
            app: 6_012 << 20,
            wired: 2_970 << 20,
            compressed: 3_420 << 20,
            cached: 2_810 << 20,
            swap_used: 1_230 << 20,
            pressure: Pressure::Normal,
        }),
        machine,
        cpu_total: tiers(&cpu_total),
        cpu_system: tiers(&cpu_system),
        memory_used: tiers(&memory_used),
        apps: Some(Arc::new(demo_apps())),
    }
}

/// Apps a working day might have open, with bundles most Macs have, for their icons.
fn demo_apps() -> Vec<AppUsage> {
    let app = |name: &str, path: &str, memory_mb: u64, cpu: f32, processes: usize| AppUsage {
        name: name.to_owned(),
        path: Some(std::path::PathBuf::from(path)),
        memory: memory_mb << 20,
        cpu,
        processes,
    };
    vec![
        app("Safari", "/Applications/Safari.app", 3_140, 0.061, 23),
        app("Xcode", "/Applications/Xcode.app", 1_870, 0.012, 6),
        app("Mail", "/System/Applications/Mail.app", 612, 0.004, 3),
        app("claude", "/usr/bin/true", 488, 0.084, 4),
        app("Music", "/System/Applications/Music.app", 402, 0.018, 2),
        app("Finder", "/System/Library/CoreServices/Finder.app", 291, 0.002, 1),
        app("Terminal", "/System/Applications/Utilities/Terminal.app", 174, 0.031, 5),
        app("Notes", "/System/Applications/Notes.app", 151, 0.001, 2),
        app("Calendar", "/System/Applications/Calendar.app", 96, 0.0, 1),
        app("Nerda Bar", "/Applications/Nerda Bar.app", 44, 0.003, 1),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_shares_come_from_tick_deltas_and_survive_a_wrap() {
        let before = [[u32::MAX - 9, 10, 100, 0], [0, 0, 0, 0]];
        let after = [[20, 30, 160, 0], [5, 5, 10, 0]];
        let cpu = Cpu::between(&before, &after, [0.0; 3]);
        // Core 0: 30 user, 20 system, 60 idle. Core 1: 5, 5, 10.
        assert!((cpu.cores[0] - 50.0 / 110.0).abs() < 1e-6);
        assert!((cpu.cores[1] - 0.5).abs() < 1e-6);
        assert!((cpu.user - 35.0 / 130.0).abs() < 1e-6);
        assert!((cpu.system - 25.0 / 130.0).abs() < 1e-6);
    }

    #[test]
    fn this_mac_reads() {
        let host = mac::Host::new();
        let machine = Machine::read();
        assert!(machine.memory > 0 && machine.performance_cores > 0);
        let ticks = host.cpu_ticks().expect("CPU ticks");
        assert_eq!(ticks.len(), machine.performance_cores + machine.efficiency_cores);
        let memory = Memory::read(&host, machine.memory).expect("memory");
        assert!(memory.used() > 0 && memory.used() <= memory.total, "{memory:?}");
    }
}
