//! The disk, the network, the battery and the GPU, read on the sampler's thread beside the CPU
//! and memory: rates from the difference of two readings of a counter, the network's use since
//! midnight, the battery's charge kept for a week on disk, and each app's disk use over the last
//! hour.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::apps::AppUsage;
use super::history::{Point, Series};
use super::net::Interface;
use super::{battery, disk, gpu, net};

/// How often the volumes are listed: it touches every mounted file system, and asks each how
/// much it could free, which takes a few milliseconds.
const VOLUMES_EVERY: f64 = 30.0;
/// A point of charge history every this many seconds, kept this long.
const CHARGE_STEP: i64 = 300;
const CHARGE_KEEP: i64 = 7 * 86_400;
const CHARGE_FILE: &str = "battery.json";
/// The window each app's disk use is summed over.
const APP_DISK_WINDOW: f64 = 3600.0;

/// The startup disk first, then every other volume, and how fast the disks read and write.
#[derive(Clone, Debug, PartialEq)]
pub struct DiskView {
    pub volumes: Vec<disk::Volume>,
    /// Bytes a second, over the time since the reading before.
    pub read: f64,
    pub write: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NetView {
    pub interfaces: Vec<Interface>,
    /// The interface carrying the default route.
    pub primary: Option<String>,
    /// Bytes a second, over the time since the reading before.
    pub down: f64,
    pub up: f64,
    /// Bytes since `since`: local midnight, or the first reading today when that came later.
    pub received_today: u64,
    pub sent_today: u64,
    pub since: i64,
}

/// One point of the battery's history.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChargePoint {
    /// Unix seconds.
    pub at: i64,
    /// From 0 to 1.
    pub charge: f32,
    pub plugged: bool,
}

/// A measure no menu bar item shows is read this often while the panel is not on its tab:
/// each costs a millisecond or more (the GPU and the battery are IOKit lookups), which a
/// reading every two seconds for another measure's item would otherwise pay too.
const QUIET_EVERY: f64 = 20.0;

/// What one reading adds to the view.
#[derive(Clone, Default)]
pub struct Readings {
    pub disk: Option<DiskView>,
    pub net: Option<NetView>,
    pub battery: Option<battery::Battery>,
    pub gpu: Option<gpu::Gpu>,
}

/// The state the readings are measured against, kept on the sampler's thread.
pub struct More {
    data_dir: Option<PathBuf>,
    disk_last: Option<(f64, disk::io::Io)>,
    /// When the network was last read, and the bytes counted by then.
    net_last: Option<(f64, (u64, u64))>,
    /// Each port's counters at the last reading, by its BSD name.
    net_ports: HashMap<String, (u64, u64)>,
    /// Bytes received and sent while this ran: each port's rises, added up. A port that comes
    /// or goes changes nothing here, where a sum of the ports' own counters would jump by all
    /// it carried since the Mac started.
    net_counted: (u64, u64),
    /// The last readings, and when each measure was read.
    latest: Readings,
    read_at: [Option<f64>; 4],
    volumes: Option<(f64, Vec<disk::Volume>)>,
    /// The local day the network counts are for, when they start, and the counters then.
    day: Option<(i32, i64, (u64, u64))>,
    pub charge: Vec<ChargePoint>,
    /// Each app's disk use per reading, oldest first, for the last hour.
    app_disk: VecDeque<(f64, HashMap<String, AppUsage>)>,
    pub disk_read: Series,
    pub disk_write: Series,
    pub net_down: Series,
    pub net_up: Series,
    pub gpu_busy: Series,
}

impl More {
    pub fn new(data_dir: Option<PathBuf>) -> More {
        let charge = data_dir.as_deref().map(load_charge).unwrap_or_default();
        More {
            data_dir,
            disk_last: None,
            net_last: None,
            net_ports: HashMap::new(),
            net_counted: (0, 0),
            latest: Readings::default(),
            read_at: [None; 4],
            volumes: None,
            day: None,
            charge,
            app_disk: VecDeque::new(),
            disk_read: Series::default(),
            disk_write: Series::default(),
            net_down: Series::default(),
            net_up: Series::default(),
            gpu_busy: Series::default(),
        }
    }

    /// Reads what `wanted` asks for (`WANT_*` bits) at `now`, and adds what held since `from`
    /// to the histories. A measure in `often` is read every time; the rest of `wanted` every
    /// [`QUIET_EVERY`] seconds, with the last reading standing in between.
    pub fn read(&mut self, from: f64, now: f64, wanted: u8, often: u8) -> Readings {
        for (at, bit) in [super::WANT_DISK, super::WANT_NETWORK, super::WANT_BATTERY, super::WANT_GPU].into_iter().enumerate() {
            if wanted & bit == 0 {
                self.read_at[at] = None;
                match at {
                    0 => self.latest.disk = None,
                    1 => self.latest.net = None,
                    2 => self.latest.battery = None,
                    _ => self.latest.gpu = None,
                }
                continue;
            }
            let last = self.read_at[at];
            if often & bit == 0 && last.is_some_and(|last| now - last < QUIET_EVERY && now >= last) {
                continue;
            }
            // What it reads now held since its own last reading, which for a quiet measure is
            // further back than the sampler's; after a long gap (the Mac slept), since `from`.
            let since = last.filter(|last| *last < from && now - last <= QUIET_EVERY * 3.0 + 30.0).unwrap_or(from);
            self.read_at[at] = Some(now);
            match at {
                0 => self.latest.disk = self.disk(since, now),
                1 => self.latest.net = self.net(since, now),
                2 => self.latest.battery = self.battery(now),
                _ => self.latest.gpu = self.gpu(since, now),
            }
        }
        self.latest.clone()
    }

    fn disk(&mut self, from: f64, now: f64) -> Option<DiskView> {
        // The first reading has nothing before it to measure a rate against.
        let measured = self.disk_last.is_some();
        let (read, write) = match (disk::io::counters(), self.disk_last) {
            (Some(io), Some((at, before))) if now > at => {
                let seconds = now - at;
                self.disk_last = Some((now, io));
                (io.read.saturating_sub(before.read) as f64 / seconds, io.written.saturating_sub(before.written) as f64 / seconds)
            }
            (Some(io), _) => {
                self.disk_last = Some((now, io));
                (0.0, 0.0)
            }
            (None, _) => (0.0, 0.0),
        };
        if measured && self.disk_last.is_some() {
            self.disk_read.add(from, now, read as f32);
            self.disk_write.add(from, now, write as f32);
        }
        let stale = self.volumes.as_ref().is_none_or(|(at, _)| now - at >= VOLUMES_EVERY);
        if stale {
            self.volumes = Some((now, disk::volumes()));
        }
        let volumes = self.volumes.as_ref().map(|(_, volumes)| volumes.clone()).unwrap_or_default();
        (!volumes.is_empty()).then_some(DiskView { volumes, read, write })
    }

    fn net(&mut self, from: f64, now: f64) -> Option<NetView> {
        let interfaces = net::interfaces();
        let totals = self.count_ports(&interfaces);
        let (down, up) = match self.net_last {
            Some((at, before)) if now > at => ((totals.0 - before.0) as f64 / (now - at), (totals.1 - before.1) as f64 / (now - at)),
            _ => (0.0, 0.0),
        };
        if self.net_last.is_some() {
            self.net_down.add(from, now, down as f32);
            self.net_up.add(from, now, up as f32);
        }
        self.net_last = Some((now, totals));
        let today = crate::time::local(now as i64).day;
        let (since, baseline) = match self.day {
            Some((day, since, baseline)) if day == today && totals.0 >= baseline.0 && totals.1 >= baseline.1 => (since, baseline),
            _ => {
                // A new day, or the first reading: count from here.
                let midnight = crate::time::local_hour_start(today, 0);
                let since = if self.day.is_some_and(|(day, ..)| day + 1 == today) { midnight } else { now as i64 };
                self.day = Some((today, since, totals));
                (since, totals)
            }
        };
        Some(NetView { primary: net::primary(), interfaces, down, up, received_today: totals.0 - baseline.0, sent_today: totals.1 - baseline.1, since })
    }

    /// Adds what each port carried since the last reading to the running count, and returns
    /// it. A port seen for the first time counts from now, and one whose counter fell (it was
    /// reset) from the reset. VPN tunnels are left out: their traffic also passes through a
    /// physical port.
    fn count_ports(&mut self, interfaces: &[Interface]) -> (u64, u64) {
        let mut ports = HashMap::with_capacity(interfaces.len());
        for port in interfaces.iter().filter(|port| port.kind != net::Kind::Vpn) {
            if let Some((received, sent)) = self.net_ports.get(&port.bsd) {
                self.net_counted.0 += port.received.checked_sub(*received).unwrap_or(port.received);
                self.net_counted.1 += port.sent.checked_sub(*sent).unwrap_or(port.sent);
            }
            ports.insert(port.bsd.clone(), (port.received, port.sent));
        }
        self.net_ports = ports;
        self.net_counted
    }

    fn battery(&mut self, now: f64) -> Option<battery::Battery> {
        let battery = battery::read()?;
        let at = now as i64;
        let due = self.charge.last().is_none_or(|last| at - last.at >= CHARGE_STEP || last.plugged != battery.plugged);
        if due {
            self.charge.push(ChargePoint { at, charge: battery.charge, plugged: battery.plugged });
            self.charge.retain(|p| at - p.at <= CHARGE_KEEP);
            if let Some(dir) = &self.data_dir {
                save_charge(dir, &self.charge);
            }
        }
        Some(battery)
    }

    fn gpu(&mut self, from: f64, now: f64) -> Option<gpu::Gpu> {
        let gpu = gpu::read()?;
        self.gpu_busy.add(from, now, gpu.busy);
        Some(gpu)
    }

    /// Adds a reading of the apps, and gives each app with its disk use over the last hour.
    pub fn app_disk(&mut self, now: f64, apps: &[AppUsage]) -> Vec<AppUsage> {
        let moved: HashMap<String, AppUsage> = apps.iter().filter(|app| app.disk > 0).map(|app| (key(app), app.clone())).collect();
        self.app_disk.push_back((now, moved));
        while self.app_disk.front().is_some_and(|(at, _)| now - at > APP_DISK_WINDOW) {
            self.app_disk.pop_front();
        }
        let mut hour: HashMap<String, AppUsage> = HashMap::new();
        for (_, reading) in &self.app_disk {
            for (key, app) in reading {
                hour.entry(key.clone()).and_modify(|sum| sum.disk += app.disk).or_insert_with(|| app.clone());
            }
        }
        // Each app as it is now, with the hour's bytes, then the ones that have since quit.
        let mut out: Vec<AppUsage> = apps
            .iter()
            .map(|app| {
                let mut app = app.clone();
                app.disk = hour.remove(&key(&app)).map_or(0, |sum| sum.disk);
                app
            })
            .collect();
        out.extend(hour.into_values().map(|mut app| {
            app.memory = 0;
            app.cpu = 0.0;
            app
        }));
        out
    }
}

/// What an app's disk use is kept under, as the apps themselves are told apart: an app
/// bundle by its path, and any other program by its name, whichever copy of it runs. Two
/// `node`s from different folders are one row in the list, and so one here.
fn key(app: &AppUsage) -> String {
    match app.path.as_ref().filter(|path| path.extension().is_some_and(|ext| ext == "app")) {
        Some(bundle) => bundle.to_string_lossy().into_owned(),
        None => app.name.clone(),
    }
}

fn load_charge(dir: &Path) -> Vec<ChargePoint> {
    std::fs::read_to_string(dir.join(CHARGE_FILE)).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

fn save_charge(dir: &Path, points: &[ChargePoint]) {
    if std::fs::create_dir_all(dir).is_ok()
        && let Ok(text) = serde_json::to_string(points)
    {
        let _ = crate::config::write_file(dir, CHARGE_FILE, &text);
    }
}

/// The tiers of a series as the view keeps them.
pub fn tiers(series: &Series, now: f64) -> [Vec<Option<Point>>; 3] {
    [0, 1, 2].map(|tier| series.points(tier, now))
}

/// A made-up disk, network, battery and GPU for headless renders: a MacBook on battery in the
/// afternoon, with a drive plugged in.
pub fn demo(now: f64, view: &mut super::SystemView) {
    let gb = 1_000_000_000u64;
    let volume = |name: &str, mount: &str, format: &str, total: u64, free: u64, available: u64, internal: bool| disk::Volume {
        name: name.to_owned(),
        mount: PathBuf::from(mount),
        format: format.to_owned(),
        total,
        free,
        available,
        internal,
        ejectable: !internal,
    };
    view.disk = Some(DiskView {
        volumes: vec![volume("Macintosh HD", "/", "APFS", 994 * gb, 341 * gb, 382 * gb, true), volume("T7 Shield", "/Volumes/T7 Shield", "ExFAT", 2000 * gb, 1400 * gb, 1400 * gb, false)],
        read: 48e6,
        write: 12e6,
    });
    let interface = |bsd: &str, name: &str, kind: net::Kind, address: Option<&str>, (received, sent): (u64, u64)| Interface {
        bsd: bsd.to_owned(),
        name: name.to_owned(),
        kind,
        address: address.map(str::to_owned),
        active: address.is_some(),
        received,
        sent,
    };
    view.net = Some(NetView {
        interfaces: vec![
            interface("en0", "Wi-Fi", net::Kind::WiFi, Some("192.168.1.24"), (4_600_000_000, 590_000_000)),
            interface("en5", "Thunderbolt Ethernet", net::Kind::Ethernet, None, (0, 0)),
            interface("utun3", "VPN", net::Kind::Vpn, Some("10.8.0.2"), (180_000_000, 26_000_000)),
        ],
        primary: Some("en0".to_owned()),
        down: 2.4e6,
        up: 380e3,
        received_today: 4_800_000_000,
        sent_today: 620_000_000,
        since: crate::time::local_hour_start(crate::time::local(now as i64).day, 0),
    });
    view.battery = Some(battery::Battery {
        charge: 0.73,
        plugged: false,
        charging: false,
        full: false,
        minutes_left: Some(220),
        watts: Some(9.4),
        cycle_count: Some(412),
        health: Some(0.89),
        condition: Some("Normal".to_owned()),
        temperature: Some(31.0),
    });
    view.gpu = Some(gpu::Gpu { cores: Some(16), busy: 0.18, rendering: 0.16, geometry: 0.09, memory: 1_900 << 20 });

    let (mut read, mut write, mut down, mut up, mut busy) = (Series::default(), Series::default(), Series::default(), Series::default(), Series::default());
    let wave = |t: f64, seed: f64| ((t * 0.0021 + seed).sin() * 0.5 + (t * 0.013 + seed * 3.0).sin() * 0.3 + (t * 0.071).sin() * 0.2) * 0.5 + 0.5;
    let mut t = now - 86_400.0;
    while t < now {
        let step = if now - t > 3600.0 { 60.0 } else { 2.0 };
        let burst = if ((t / 2.0) as i64 % 211) < 4 { 1.0 } else { 0.0 };
        read.add(t, t + step, (6e6 + wave(t, 0.7) * 10e6 + burst * 80e6) as f32);
        write.add(t, t + step, (3e6 + wave(t, 1.9) * 6e6) as f32);
        down.add(t, t + step, (1.2e6 + wave(t, 0.2) * 2.4e6 + burst * 4e6) as f32);
        up.add(t, t + step, (0.2e6 + wave(t, 2.4) * 0.5e6) as f32);
        busy.add(t, t + step, (0.08 + wave(t, 1.1).powf(3.0) * 0.4 + burst * 0.3) as f32);
        t += step;
    }
    view.disk_read = tiers(&read, now);
    view.disk_write = tiers(&write, now);
    view.net_down = tiers(&down, now);
    view.net_up = tiers(&up, now);
    view.gpu_busy = tiers(&busy, now);
    // A day of charge: down through the afternoon, plugged in overnight, down again since.
    let mut charge = Vec::new();
    let start = now as i64 - 7 * 86_400;
    let mut at = start;
    while at <= now as i64 {
        let hour = ((at - start) / 3600) % 24;
        let plugged = (11..15).contains(&hour);
        let level = if plugged { 0.55 + (hour - 11) as f32 * 0.15 } else { (1.0 - ((hour + 9) % 24) as f32 * 0.035).clamp(0.2, 1.0) };
        charge.push(ChargePoint { at, charge: level.min(1.0), plugged });
        at += CHARGE_STEP * 3;
    }
    view.charge = std::sync::Arc::new(charge);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, disk: u64) -> AppUsage {
        AppUsage { name: name.to_owned(), path: None, memory: 1, cpu: 0.0, disk, processes: 1 }
    }

    #[test]
    fn an_apps_disk_use_sums_over_the_last_hour() {
        let mut more = More::new(None);
        // A priming reading and the one right after it both count.
        more.app_disk(0.0, &[app("Xcode", 700), app("Mail", 5)]);
        let now = more.app_disk(0.3, &[app("Xcode", 300), app("Mail", 0)]);
        let disk = |apps: &[AppUsage], name: &str| apps.iter().find(|a| a.name == name).map(|a| a.disk);
        assert_eq!(disk(&now, "Xcode"), Some(1000));
        assert_eq!(disk(&now, "Mail"), Some(5));
        // An app that quit still counts until its hour is up, then falls away.
        let later = more.app_disk(1800.0, &[app("Mail", 0)]);
        assert_eq!(disk(&later, "Xcode"), Some(1000));
        let much_later = more.app_disk(3700.0, &[app("Mail", 0)]);
        assert_eq!(disk(&much_later, "Xcode"), None);
    }

    #[test]
    fn a_tool_run_from_two_folders_is_one_app() {
        let mut more = More::new(None);
        let node = |path: &str, disk: u64| AppUsage { path: Some(PathBuf::from(path)), ..app("node", disk) };
        more.app_disk(0.0, &[node("/opt/homebrew/bin/node", 100)]);
        // The newest `node` process is now another copy: the row keeps what the first one moved.
        let now = more.app_disk(20.0, &[node("/Users/me/.local/bin/node", 50)]);
        assert_eq!(now.iter().map(|a| (a.name.as_str(), a.disk)).collect::<Vec<_>>(), [("node", 150)]);
        // Two apps of one name in different bundles stay apart.
        let bundle = |path: &str| AppUsage { path: Some(PathBuf::from(path)), ..app("Notes", 1) };
        assert_ne!(key(&bundle("/Applications/Notes.app")), key(&bundle("/System/Applications/Notes.app")));
    }

    #[test]
    fn a_port_that_comes_or_goes_adds_nothing_of_its_own() {
        let port = |bsd: &str, kind: net::Kind, received: u64, sent: u64| Interface { bsd: bsd.to_owned(), name: bsd.to_owned(), kind, address: None, active: true, received, sent };
        let mut more = More::new(None);
        // The first reading counts from now, whatever the ports carried since the Mac started.
        assert_eq!(more.count_ports(&[port("en0", net::Kind::WiFi, 5_000, 900)]), (0, 0));
        // A wired port is plugged in with 593 MB already on its counters: none of it is new.
        assert_eq!(more.count_ports(&[port("en0", net::Kind::WiFi, 5_400, 1_000), port("en6", net::Kind::Ethernet, 593_000_000, 7_000)]), (400, 100));
        assert_eq!(more.count_ports(&[port("en0", net::Kind::WiFi, 5_500, 1_000), port("en6", net::Kind::Ethernet, 593_000_300, 7_050)]), (800, 150));
        // Unplugged, it leaves the list: the count doesn't fall.
        assert_eq!(more.count_ports(&[port("en0", net::Kind::WiFi, 5_600, 1_000)]), (900, 150));
        // A counter that starts over counts from its reset, and a VPN's traffic isn't counted twice.
        assert_eq!(more.count_ports(&[port("en0", net::Kind::WiFi, 40, 10), port("utun4", net::Kind::Vpn, 9_999, 9_999)]), (940, 160));
    }

    #[test]
    fn a_measure_no_item_shows_is_read_less_often() {
        let mut more = More::new(None);
        let wanted = crate::system::WANT_GPU | crate::system::WANT_BATTERY;
        more.read(0.0, 2.0, wanted, 0);
        assert_eq!(more.read_at[3], Some(2.0));
        more.read(2.0, 4.0, wanted, 0);
        assert_eq!(more.read_at[3], Some(2.0), "two seconds on, the last reading stands");
        more.read(4.0, 6.0, wanted, crate::system::WANT_GPU);
        assert_eq!((more.read_at[3], more.read_at[2]), (Some(6.0), Some(2.0)), "in the menu bar it is read every time");
        more.read(20.0, 22.0, wanted, 0);
        assert_eq!(more.read_at[2], Some(22.0), "and after its twenty seconds either way");
        more.read(22.0, 24.0, crate::system::WANT_GPU, 0);
        assert!(more.read_at[2].is_none() && more.latest.battery.is_none(), "a measure nothing shows is dropped");
    }
}
