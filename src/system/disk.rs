//! The Mac's volumes as Finder shows them: the boot disk ("Macintosh HD") first, then every
//! drive and disk image under `/Volumes`, with how full each is; and ejecting one. [`io`] below
//! counts what the disks read and write.
//!
//! On APFS the boot disk is two volumes: `/` is a sealed, read-only snapshot of the system,
//! and the files live on `/System/Volumes/Data` beside it. Both share one container's space,
//! so the boot disk shows as one entry with the name of `/` and the figures of the container,
//! which is what `statfs` on the data volume reports.
//!
//! Space "available" counts what macOS would free on demand (caches, local snapshots, iCloud
//! copies), as Finder does; "free" is what is free right now. The difference is purgeable.

use std::ffi::{CStr, CString, c_int};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::{NSArray, NSNumber, NSString, NSURL, NSURLResourceKey};

/// One volume and how full it is.
#[derive(Clone, Debug, PartialEq)]
pub struct Volume {
    /// "Macintosh HD", "T7 Shield".
    pub name: String,
    pub mount: PathBuf,
    /// "APFS", "ExFAT", "Mac OS Extended".
    pub format: String,
    /// Bytes.
    pub total: u64,
    /// Bytes free right now.
    pub free: u64,
    /// Bytes free once macOS clears what it can on demand, as Finder counts them; at least
    /// `free` in practice.
    pub available: u64,
    /// The boot disk, or another volume on a disk inside the Mac.
    pub internal: bool,
    /// A removable or external drive, a disk image or a server, which can be ejected.
    pub ejectable: bool,
}

#[allow(dead_code)]
impl Volume {
    /// Bytes in use, not counting what macOS could clear.
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.available)
    }

    /// Bytes macOS can clear on demand.
    pub fn purgeable(&self) -> u64 {
        self.available.saturating_sub(self.free)
    }
}

/// Where the boot disk's files live.
const DATA: &str = "/System/Volumes/Data";

/// The boot disk first, then every other volume Finder shows.
#[allow(dead_code)]
pub fn volumes() -> Vec<Volume> {
    // The reader thread has no autorelease pool of its own; Foundation's leftovers go here.
    objc2::rc::autoreleasepool(|_| {
        let mut list: Vec<Volume> = boot().into_iter().collect();
        list.extend(mounts().into_iter().filter(shown).map(volume));
        list
    })
}

/// Ejects a volume that can be ejected, as Finder's eject button does; a server is unmounted.
/// The error is what `diskutil` said.
#[allow(dead_code)]
pub fn eject(mount: &Path) -> Result<(), String> {
    let Some(volume) = volumes().into_iter().find(|v| v.mount == mount) else {
        return Err(format!("{} is not a mounted volume", mount.display()));
    };
    if !volume.ejectable {
        return Err(format!("{} can't be ejected", volume.name));
    }
    let local = stat(mount).is_some_and(|m| m.flags & libc::MNT_LOCAL as u32 != 0);
    let output = Command::new("/usr/sbin/diskutil")
        .arg(if local { "eject" } else { "unmount" })
        .arg(mount)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Couldn't run diskutil: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let said = |bytes: &[u8]| String::from_utf8_lossy(bytes).trim().to_owned();
    let error = [said(&output.stderr), said(&output.stdout)].into_iter().find(|s| !s.is_empty());
    Err(error.unwrap_or_else(|| format!("diskutil failed ({})", output.status)))
}

/// A mounted file system as `statfs` describes it.
#[derive(Clone, Debug)]
struct Mount {
    path: PathBuf,
    /// "apfs", "exfat".
    fstype: String,
    flags: u32,
    total: u64,
    free: u64,
}

impl Mount {
    fn new(fs: &libc::statfs) -> Mount {
        // SAFETY: the kernel nul-terminates both names within their arrays.
        let text = |name: &[libc::c_char]| unsafe { CStr::from_ptr(name.as_ptr()) };
        let block = fs.f_bsize as u64;
        Mount {
            path: PathBuf::from(std::ffi::OsStr::from_bytes(text(&fs.f_mntonname).to_bytes())),
            fstype: text(&fs.f_fstypename).to_string_lossy().into_owned(),
            flags: fs.f_flags,
            total: fs.f_blocks.saturating_mul(block),
            free: fs.f_bavail.saturating_mul(block),
        }
    }
}

unsafe extern "C" {
    /// `getmntinfo` into a buffer of the caller's own, which the caller frees; plain
    /// `getmntinfo` shares one buffer across threads.
    #[cfg_attr(target_arch = "x86_64", link_name = "getmntinfo_r_np$INODE64")]
    fn getmntinfo_r_np(list: *mut *mut libc::statfs, flags: c_int) -> c_int;
}

/// Every mounted file system, as the kernel last knew them (without asking each to update,
/// so a server that stopped answering can't hold the reading up).
fn mounts() -> Vec<Mount> {
    let mut list: *mut libc::statfs = std::ptr::null_mut();
    // SAFETY: on success `list` points at `count` entries in a buffer this code owns, which
    // is copied out and then freed.
    unsafe {
        let count = getmntinfo_r_np(&mut list, libc::MNT_NOWAIT);
        if list.is_null() {
            return Vec::new();
        }
        let mounts = std::slice::from_raw_parts(list, count.max(0) as usize).iter().map(Mount::new).collect();
        libc::free(list as *mut libc::c_void);
        mounts
    }
}

/// One mounted file system, by its path.
fn stat(path: &Path) -> Option<Mount> {
    let path = CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: fills the zeroed struct for a nul-terminated path.
    unsafe {
        let mut fs: libc::statfs = std::mem::zeroed();
        (libc::statfs(path.as_ptr(), &mut fs) == 0).then(|| Mount::new(&fs))
    }
}

/// Names of the disk images macOS mounts for its own updates and installers.
const UPDATE_IMAGES: [&str; 5] = ["Shared Support", "macOS Base System", "Update", "MobileSoftwareUpdate", "InstallESD"];

/// Whether Finder would show a volume other than the boot disk: one under `/Volumes` that isn't
/// hidden from browsing, a snapshot, a pseudo file system or an update's disk image.
fn shown(mount: &Mount) -> bool {
    let hidden = (libc::MNT_DONTBROWSE | libc::MNT_SNAPSHOT) as u32;
    let Ok(name) = mount.path.strip_prefix("/Volumes") else { return false };
    let name = name.to_string_lossy();
    mount.flags & hidden == 0
        && !["devfs", "autofs", "nullfs"].contains(&mount.fstype.as_str())
        && !name.is_empty()
        && !name.contains('/')
        && !name.starts_with('.')
        && !UPDATE_IMAGES.contains(&name.as_ref())
}

/// A file system type as Disk Utility names it.
fn format_name(fstype: &str) -> String {
    match fstype {
        "apfs" => "APFS",
        "hfs" => "Mac OS Extended",
        "exfat" => "ExFAT",
        "msdos" => "FAT",
        "ntfs" => "NTFS",
        "smbfs" => "SMB",
        "afpfs" => "AFP",
        "nfs" => "NFS",
        "webdav" => "WebDAV",
        "cd9660" => "ISO 9660",
        "udf" => "UDF",
        other => return other.to_uppercase(),
    }
    .to_owned()
}

/// What Foundation knows about a volume that `statfs` doesn't.
#[derive(Default)]
struct Facts {
    name: Option<String>,
    available: Option<u64>,
    internal: Option<bool>,
    ejectable: bool,
}

/// Asks Foundation about the volume a path is on. The available space is worked out fresh on
/// every call, which can take a few milliseconds.
fn facts(path: &Path) -> Facts {
    let Some(text) = path.to_str() else { return Facts::default() };
    let url = NSURL::fileURLWithPath(&NSString::from_str(text));
    // SAFETY: these are Foundation's own constant strings, valid for the life of the process.
    let keys: [&NSURLResourceKey; 5] = unsafe {
        use objc2_foundation::*;
        [NSURLVolumeLocalizedNameKey, NSURLVolumeAvailableCapacityForImportantUsageKey, NSURLVolumeIsInternalKey, NSURLVolumeIsEjectableKey, NSURLVolumeIsRemovableKey]
    };
    let Ok(values) = url.resourceValuesForKeys_error(&NSArray::from_slice(&keys)) else { return Facts::default() };
    let value = |key: &NSURLResourceKey| values.objectForKey(key);
    let number = |key| value(key).and_then(|v: Retained<AnyObject>| v.downcast::<NSNumber>().ok());
    let flag = |key| number(key).map(|n| n.boolValue());
    Facts {
        name: value(keys[0]).and_then(|v| v.downcast::<NSString>().ok()).map(|s| s.to_string()).filter(|s| !s.is_empty()),
        available: number(keys[1]).map(|n| n.longLongValue()).filter(|&n| n > 0).map(|n| n as u64),
        internal: flag(keys[2]),
        ejectable: flag(keys[3]).unwrap_or(false) || flag(keys[4]).unwrap_or(false),
    }
}

/// The boot disk: named for `/`, sized as the container its data volume shares.
fn boot() -> Option<Volume> {
    let root = stat(Path::new("/"))?;
    let data = stat(Path::new(DATA)).filter(|d| d.total > 0).unwrap_or_else(|| root.clone());
    let root_facts = facts(Path::new("/"));
    let sane = |a: &u64| *a >= data.free && *a <= data.total;
    let available = root_facts.available.filter(sane).or_else(|| facts(Path::new(DATA)).available.filter(sane));
    Some(Volume {
        name: root_facts.name.unwrap_or_else(|| "Macintosh HD".to_owned()),
        mount: PathBuf::from("/"),
        format: format_name(&root.fstype),
        total: data.total,
        free: data.free,
        available: available.unwrap_or(data.free),
        internal: true,
        ejectable: false,
    })
}

/// A volume other than the boot disk. A server is asked nothing more, so one that stopped
/// answering can't hold the reading up.
fn volume(mount: Mount) -> Volume {
    let local = mount.flags & libc::MNT_LOCAL as u32 != 0;
    let facts = if local { facts(&mount.path) } else { Facts::default() };
    let internal = local && facts.internal.unwrap_or(false);
    let last = mount.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Volume {
        name: facts.name.unwrap_or(last),
        format: format_name(&mount.fstype),
        total: mount.total,
        free: mount.free,
        available: facts.available.filter(|&a| a >= mount.free).unwrap_or(mount.free),
        internal,
        ejectable: !local || facts.ejectable || !internal,
        mount: mount.path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_this_macs_volumes() {
        let started = std::time::Instant::now();
        let list = volumes();
        let first = started.elapsed();
        volumes();
        eprintln!("volumes read in {first:?}, then in {:?}", started.elapsed() - first);
        for v in &list {
            let gb = |b: u64| b as f64 / 1e9;
            eprintln!(
                "{:<16} {:<24} {:<8} total {:.1} GB, used {:.1}, free {:.1}, available {:.1}, purgeable {:.1}, internal={} ejectable={}",
                v.name, v.mount.display(), v.format, gb(v.total), gb(v.used()), gb(v.free), gb(v.available), gb(v.purgeable()), v.internal, v.ejectable
            );
        }
        for path in ["/", DATA] {
            eprintln!("important-usage space on {path}: {:?}", facts(Path::new(path)).available);
        }
        let boot = &list[0];
        assert_eq!(boot.mount, Path::new("/"));
        assert!(boot.internal && !boot.ejectable && boot.total > 0 && boot.free > 0);
        assert!(boot.available >= boot.free && boot.available <= boot.total);
        assert_eq!(list.iter().filter(|v| v.mount == Path::new("/")).count(), 1);
        assert!(list.iter().all(|v| !v.mount.starts_with("/System/Volumes")));
    }

    #[test]
    fn refuses_to_eject_the_boot_disk() {
        assert!(eject(Path::new("/")).is_err());
        assert!(eject(Path::new("/Volumes/Not Mounted Here")).is_err());
    }

    fn mount(path: &str, fstype: &str, flags: c_int) -> Mount {
        Mount { path: PathBuf::from(path), fstype: fstype.to_owned(), flags: flags as u32, total: 1, free: 1 }
    }

    #[test]
    fn shows_what_finder_shows() {
        let (local, nobrowse) = (libc::MNT_LOCAL, libc::MNT_LOCAL | libc::MNT_DONTBROWSE);
        assert!(shown(&mount("/Volumes/T7 Shield", "exfat", local)));
        assert!(shown(&mount("/Volumes/Backup", "hfs", local | libc::MNT_JOURNALED)));
        assert!(shown(&mount("/Volumes/share", "smbfs", 0)));
        assert!(!shown(&mount("/", "apfs", local | libc::MNT_RDONLY | libc::MNT_SNAPSHOT)));
        assert!(!shown(&mount("/dev", "devfs", nobrowse)));
        assert!(!shown(&mount("/System/Volumes/Data", "apfs", nobrowse)));
        assert!(!shown(&mount("/System/Volumes/VM", "apfs", local)));
        assert!(!shown(&mount("/Volumes/Recovery", "apfs", nobrowse)));
        assert!(!shown(&mount("/private/var/vm", "apfs", local)));
        assert!(!shown(&mount("/Volumes/.timemachine/abc/2026-11-16", "apfs", local)));
        assert!(!shown(&mount("/Volumes/Shared Support", "apfs", local | libc::MNT_RDONLY)));
        assert!(shown(&mount("/Volumes/Updates 2026", "apfs", local)));
        assert!(!shown(&mount("/Volumes/auto", "autofs", 0)));
    }

    #[test]
    fn names_formats() {
        assert_eq!(format_name("apfs"), "APFS");
        assert_eq!(format_name("exfat"), "ExFAT");
        assert_eq!(format_name("msdos"), "FAT");
        assert_eq!(format_name("hfs"), "Mac OS Extended");
        assert_eq!(format_name("lifs"), "LIFS");
    }
}

/// How much every disk has read and written since it was attached, from the counters each
/// `IOBlockStorageDriver` keeps; two readings apart give the rate.
pub mod io {
    use super::super::iokit;

    /// Cumulative bytes across every disk, disk images and external drives included.
    #[allow(dead_code)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct Io {
        pub read: u64,
        pub written: u64,
    }

    /// The counters now; `None` when no disk reports any.
    #[allow(dead_code)]
    pub fn counters() -> Option<Io> {
        let mut total = None::<Io>;
        for service in iokit::services(c"IOBlockStorageDriver") {
            let Some(properties) = service.properties() else { continue };
            let Some(stats) = properties.dict().and_then(|d| d.dict("Statistics")) else { continue };
            let bytes = |name: &str| stats.i64(name).unwrap_or(0).max(0) as u64;
            let io = total.get_or_insert_default();
            io.read = io.read.wrapping_add(bytes("Bytes (Read)"));
            io.written = io.written.wrapping_add(bytes("Bytes (Write)"));
        }
        total
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn reads_this_macs_disks() {
            let Some(first) = counters() else {
                eprintln!("disk io: no disk reports counters");
                return;
            };
            let second = counters().unwrap();
            eprintln!("disk io: {first:?}, then {second:?}");
            assert!(first.read > 0);
            assert!(second.read >= first.read && second.written >= first.written);
        }
    }
}
