//! The Mac's network ports and the traffic through them: the Wi-Fi and wired ports System
//! Settings lists, even while unplugged, plus any VPN tunnel while it carries an address.
//!
//! What each port is called comes from SystemConfiguration, the names System Settings shows,
//! read at most once a minute. A reading itself is `getifaddrs` for the addresses and a
//! `sysctl` per shown port (`IFMIB_IFDATA`) for its traffic, the source `netstat -ib` uses.
//! Neither of the obvious sources will do for the traffic: `getifaddrs` counts in 32 bits and
//! wraps every 4 GiB, and `NET_RT_IFLIST2`, though 64-bit in its layout, now comes back cut to
//! 32 bits and rounded to the KiB.

use std::collections::HashMap;
use std::ffi::{CStr, c_int, c_void};
use std::mem::size_of;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::ptr::{null_mut, read_unaligned};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// What sort of port an interface is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    WiFi,
    Ethernet,
    Vpn,
    Other,
}

/// One network port, as it is now.
#[derive(Clone, Debug, PartialEq)]
pub struct Interface {
    /// "en0".
    pub bsd: String,
    /// "Wi-Fi", "Thunderbolt Ethernet", "VPN": the service's name in System Settings.
    pub name: String,
    pub kind: Kind,
    /// Its IPv4 address.
    pub address: Option<String>,
    /// Up, running and with an address: an IPv4 one, or an IPv6 one beyond the link-local
    /// address every port gives itself. A VPN counts only with an IPv4 address.
    pub active: bool,
    /// Bytes in since the Mac started.
    pub received: u64,
    /// Bytes out since the Mac started.
    pub sent: u64,
}

/// How long the names from System Settings are kept before they are read again.
const NAMES_FOR: Duration = Duration::from_secs(60);

/// A network service from System Settings: its port and what the user sees it called.
#[derive(Clone, Debug)]
struct Service {
    bsd: String,
    name: String,
    kind: Kind,
}

static SERVICES: Mutex<Option<(Instant, Vec<Service>)>> = Mutex::new(None);

/// The services, read again when the last reading is a minute old. A reading that fails keeps
/// the names from before.
fn services() -> Vec<Service> {
    let mut cache = SERVICES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if !cache.as_ref().is_some_and(|(at, _)| at.elapsed() < NAMES_FOR) {
        let read = objc2::rc::autoreleasepool(|_| sc::services()).filter(|list| !list.is_empty());
        let old = cache.take().map(|(_, list)| list);
        *cache = Some((Instant::now(), read.or(old).unwrap_or_default()));
    }
    cache.as_ref().map(|(_, list)| list.clone()).unwrap_or_default()
}

/// The Mac's network ports: each Wi-Fi and wired service in System Settings' order, whether
/// connected or not, then each VPN tunnel that has an IPv4 address.
#[allow(dead_code)]
pub fn interfaces() -> Vec<Interface> {
    let links = links();
    let make = |bsd: &str, name: String, kind: Kind, link: &Link| {
        let (received, sent) = counters(link.index).unwrap_or((0, 0));
        let active = link.running && (link.ipv4.is_some() || (kind != Kind::Vpn && link.routable_v6));
        Interface { bsd: bsd.to_owned(), name, kind, address: link.ipv4.map(|a| a.to_string()), active, received, sent }
    };
    let mut list: Vec<Interface> = services()
        .into_iter()
        .filter(|s| !skipped(&s.bsd) && !is_vpn(&s.bsd))
        .filter_map(|s| links.get(&s.bsd).map(|link| make(&s.bsd, s.name, s.kind, link)))
        .filter(|i| i.active || matches!(i.kind, Kind::WiFi | Kind::Ethernet))
        .collect();
    let mut tunnels: Vec<_> = links.iter().filter(|(bsd, link)| is_vpn(bsd) && link.running && link.ipv4.is_some()).collect();
    tunnels.sort_by(|a, b| a.0.cmp(b.0));
    list.extend(tunnels.into_iter().map(|(bsd, link)| make(bsd, "VPN".to_owned(), Kind::Vpn, link)));
    list
}

/// Bytes received and sent over every port but VPN tunnels, whose traffic also passes through
/// a physical port and would otherwise count twice.
#[allow(dead_code)]
pub fn totals(interfaces: &[Interface]) -> (u64, u64) {
    interfaces.iter().filter(|i| i.kind != Kind::Vpn).fold((0, 0), |(r, s), i| (r + i.received, s + i.sent))
}

/// The port the default IPv4 route leaves through, as its BSD name: the one the Mac is online
/// through, as macOS itself picked it (`State:/Network/Global/IPv4`, `PrimaryInterface`).
#[allow(dead_code)]
pub fn primary() -> Option<String> {
    sc::primary()
}

/// Ports that are never shown: loopback, Apple's peer-to-peer links, bridges, the USB-C ports'
/// internal links, the Mac's own hotspot, and the IPv6 tunnelling stubs.
fn skipped(bsd: &str) -> bool {
    ["lo", "awdl", "llw", "bridge", "anpi", "ap", "gif", "stf"].iter().any(|p| bsd.starts_with(p))
}

/// A VPN tunnel's port.
fn is_vpn(bsd: &str) -> bool {
    ["utun", "ipsec", "ppp"].iter().any(|p| bsd.starts_with(p))
}

/// The kind of a SystemConfiguration interface type; `None` for one that is never shown, such
/// as a bridge, whose members' traffic is already counted on its own ports.
fn kind_of(interface_type: &str) -> Option<Kind> {
    match interface_type {
        "IEEE80211" => Some(Kind::WiFi),
        "Ethernet" | "Bond" => Some(Kind::Ethernet),
        "Bridge" | "VLAN" | "6to4" | "Loopback" => None,
        _ => Some(Kind::Other),
    }
}

/// An IPv6 address that reaches past the port's own link.
fn routable(address: Ipv6Addr) -> bool {
    !address.is_loopback() && !address.is_unspecified() && address.segments()[0] & 0xffc0 != 0xfe80
}

/// What `getifaddrs` says about one port.
#[derive(Default)]
struct Link {
    index: u16,
    running: bool,
    ipv4: Option<Ipv4Addr>,
    routable_v6: bool,
}

/// Every port's flags and addresses, by BSD name.
fn links() -> HashMap<String, Link> {
    let mut links: HashMap<String, Link> = HashMap::new();
    let mut list: *mut libc::ifaddrs = null_mut();
    // SAFETY: getifaddrs allocates a list, which is walked and then freed. Each address is read
    // as the family it says it is, without assuming its alignment.
    unsafe {
        if libc::getifaddrs(&mut list) != 0 {
            return links;
        }
        let mut at = list;
        while let Some(entry) = at.as_ref() {
            at = entry.ifa_next;
            let link = links.entry(CStr::from_ptr(entry.ifa_name).to_string_lossy().into_owned()).or_default();
            let flags = entry.ifa_flags as c_int;
            link.running = flags & libc::IFF_UP != 0 && flags & libc::IFF_RUNNING != 0;
            let Some(address) = entry.ifa_addr.as_ref() else { continue };
            match address.sa_family as c_int {
                libc::AF_LINK => link.index = read_unaligned((entry.ifa_addr as *const u8).add(2) as *const u16),
                libc::AF_INET => {
                    let ip = read_unaligned(entry.ifa_addr as *const libc::sockaddr_in);
                    link.ipv4.get_or_insert(Ipv4Addr::from(u32::from_be(ip.sin_addr.s_addr)));
                }
                libc::AF_INET6 => {
                    let ip = read_unaligned(entry.ifa_addr as *const libc::sockaddr_in6);
                    link.routable_v6 |= routable(Ipv6Addr::from(ip.sin6_addr.s6_addr));
                }
                _ => {}
            }
        }
        libc::freeifaddrs(list);
    }
    links
}

/// One port's 64-bit byte counters, in and out, by its interface index.
fn counters(index: u16) -> Option<(u64, u64)> {
    let mut mib = [libc::CTL_NET, libc::PF_LINK, libc::NETLINK_GENERIC, libc::IFMIB_IFDATA, index as c_int, libc::IFDATA_GENERAL];
    // SAFETY: the kernel writes at most `size` bytes into the zeroed struct.
    let data = unsafe {
        let mut data: libc::ifmibdata = std::mem::zeroed();
        let mut size = size_of::<libc::ifmibdata>();
        let ok = libc::sysctl(mib.as_mut_ptr(), mib.len() as u32, &mut data as *mut _ as *mut c_void, &mut size, null_mut(), 0) == 0;
        (ok && index != 0 && size == size_of::<libc::ifmibdata>()).then_some(data.ifmd_data)?
    };
    Some((data.ifi_ibytes, data.ifi_obytes))
}

/// The SystemConfiguration side: the network services of the current location.
mod sc {
    use super::{Service, kind_of};
    use std::ffi::{c_char, c_void};
    use std::ptr::null;

    type Ref = *const c_void;
    const UTF8: u32 = 0x0800_0100;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(object: Ref);
        fn CFArrayGetCount(array: Ref) -> isize;
        fn CFArrayGetValueAtIndex(array: Ref, index: isize) -> Ref;
        fn CFStringCreateWithCString(allocator: Ref, text: *const c_char, encoding: u32) -> Ref;
        fn CFStringGetLength(string: Ref) -> isize;
        fn CFStringGetMaximumSizeForEncoding(length: isize, encoding: u32) -> isize;
        fn CFStringGetCString(string: Ref, buffer: *mut c_char, size: isize, encoding: u32) -> bool;
        fn CFDictionaryGetValue(dictionary: Ref, key: Ref) -> Ref;
        fn CFBooleanGetValue(boolean: Ref) -> bool;
        fn CFGetTypeID(object: Ref) -> usize;
        fn CFBooleanGetTypeID() -> usize;
        fn CFDictionaryGetTypeID() -> usize;
        fn CFStringGetTypeID() -> usize;
    }

    #[link(name = "SystemConfiguration", kind = "framework")]
    unsafe extern "C" {
        fn SCPreferencesCreate(allocator: Ref, name: Ref, prefs_id: Ref) -> Ref;
        fn SCPreferencesPathGetValue(prefs: Ref, path: Ref) -> Ref;
        fn SCDynamicStoreCreate(allocator: Ref, name: Ref, callout: Ref, context: Ref) -> Ref;
        fn SCDynamicStoreCopyValue(store: Ref, key: Ref) -> Ref;
        fn SCNetworkSetCopyCurrent(prefs: Ref) -> Ref;
        fn SCNetworkSetCopyServices(set: Ref) -> Ref;
        fn SCNetworkSetGetServiceOrder(set: Ref) -> Ref;
        fn SCNetworkServiceGetEnabled(service: Ref) -> u8;
        fn SCNetworkServiceGetInterface(service: Ref) -> Ref;
        fn SCNetworkServiceGetName(service: Ref) -> Ref;
        fn SCNetworkServiceGetServiceID(service: Ref) -> Ref;
        fn SCNetworkInterfaceGetBSDName(interface: Ref) -> Ref;
        fn SCNetworkInterfaceGetInterfaceType(interface: Ref) -> Ref;
        fn SCNetworkInterfaceGetLocalizedDisplayName(interface: Ref) -> Ref;
    }

    /// A Core Foundation object from a Create or Copy call, released when dropped.
    struct Owned(Ref);

    impl Owned {
        fn new(object: Ref) -> Option<Owned> {
            // `then`, not `then_some`: an `Owned` built from null would be dropped, and release it.
            (!object.is_null()).then(|| Owned(object))
        }
    }

    impl Drop for Owned {
        fn drop(&mut self) {
            // SAFETY: this code owns the one reference a Create or Copy call handed it.
            unsafe { CFRelease(self.0) };
        }
    }

    /// A CFString as a Rust string.
    ///
    /// # Safety
    /// `string` is null or a live CFString.
    unsafe fn string(string: Ref) -> Option<String> {
        if string.is_null() {
            return None;
        }
        // SAFETY: the buffer is as large as CF says the string can need, nul included.
        unsafe {
            let size = CFStringGetMaximumSizeForEncoding(CFStringGetLength(string), UTF8) + 1;
            let mut buffer = vec![0 as c_char; size.max(1) as usize];
            CFStringGetCString(string, buffer.as_mut_ptr(), size, UTF8).then(|| std::ffi::CStr::from_ptr(buffer.as_ptr()).to_string_lossy().into_owned())
        }
    }

    /// A new CFString.
    fn cf_string(text: &str) -> Option<Owned> {
        let text = std::ffi::CString::new(text).ok()?;
        // SAFETY: copies the nul-terminated text into a string this code then owns.
        Owned::new(unsafe { CFStringCreateWithCString(null(), text.as_ptr(), UTF8) })
    }

    /// Whether macOS keeps a service out of System Settings: the USB-C ports' internal
    /// Ethernet links ("Ethernet Adapter (en5)") have services marked this way.
    ///
    /// # Safety
    /// `prefs` is live.
    unsafe fn hidden(prefs: Ref, id: &str, key: &Owned) -> bool {
        let Some(path) = cf_string(&format!("/NetworkServices/{id}/Interface")) else { return false };
        // SAFETY: both results are borrowed from `prefs`; the value is read only if it is a
        // CFBoolean, which a stored `true` is.
        unsafe {
            let interface = SCPreferencesPathGetValue(prefs, path.0);
            if interface.is_null() {
                return false;
            }
            let value = CFDictionaryGetValue(interface, key.0);
            !value.is_null() && CFGetTypeID(value) == CFBooleanGetTypeID() && CFBooleanGetValue(value)
        }
    }

    /// The primary interface's BSD name from the dynamic store, where configd keeps the
    /// network's live state.
    pub fn primary() -> Option<String> {
        let (name, key, field) = (cf_string("Nerda Bar")?, cf_string("State:/Network/Global/IPv4")?, cf_string("PrimaryInterface")?);
        // SAFETY: the store and the copied value are owned and released once; the interface's
        // name is borrowed from the value, and each is read only as the type it says it is.
        unsafe {
            let store = Owned::new(SCDynamicStoreCreate(null(), name.0, null(), null()))?;
            let state = Owned::new(SCDynamicStoreCopyValue(store.0, key.0))?;
            if CFGetTypeID(state.0) != CFDictionaryGetTypeID() {
                return None;
            }
            let primary = CFDictionaryGetValue(state.0, field.0);
            (!primary.is_null() && CFGetTypeID(primary) == CFStringGetTypeID()).then(|| string(primary))?
        }
    }

    /// A CFArray's items, borrowed from it.
    ///
    /// # Safety
    /// `array` is null or a live CFArray, which must outlive the items.
    unsafe fn items(array: Ref) -> Vec<Ref> {
        if array.is_null() {
            return Vec::new();
        }
        // SAFETY: every index below the count is in the array.
        unsafe { (0..CFArrayGetCount(array)).map(|i| CFArrayGetValueAtIndex(array, i)).collect() }
    }

    /// The current location's enabled services that have a port, in System Settings' order;
    /// `None` when the preferences can't be read.
    pub fn services() -> Option<Vec<Service>> {
        // SAFETY: each Create and Copy result is owned by an `Owned` and released once; Get
        // results and array items are borrowed from objects that outlive their use here.
        unsafe {
            let name = cf_string("Nerda Bar")?;
            let hidden_key = cf_string("HiddenConfiguration")?;
            let prefs = Owned::new(SCPreferencesCreate(null(), name.0, null()))?;
            let set = Owned::new(SCNetworkSetCopyCurrent(prefs.0))?;
            let all = Owned::new(SCNetworkSetCopyServices(set.0))?;
            let order: Vec<String> = items(SCNetworkSetGetServiceOrder(set.0)).into_iter().filter_map(|id| string(id)).collect();
            let mut ranked = Vec::new();
            for service in items(all.0) {
                let interface = SCNetworkServiceGetInterface(service);
                if SCNetworkServiceGetEnabled(service) == 0 || interface.is_null() {
                    continue;
                }
                let Some(bsd) = string(SCNetworkInterfaceGetBSDName(interface)) else { continue };
                let Some(kind) = kind_of(&string(SCNetworkInterfaceGetInterfaceType(interface)).unwrap_or_default()) else { continue };
                let name = string(SCNetworkServiceGetName(service)).or_else(|| string(SCNetworkInterfaceGetLocalizedDisplayName(interface))).unwrap_or_else(|| bsd.clone());
                let id = string(SCNetworkServiceGetServiceID(service)).unwrap_or_default();
                if hidden(prefs.0, &id, &hidden_key) {
                    continue;
                }
                let rank = order.iter().position(|o| *o == id).unwrap_or(usize::MAX);
                ranked.push((rank, Service { bsd, name, kind }));
            }
            ranked.sort_by_key(|(rank, _)| *rank);
            Some(ranked.into_iter().map(|(_, service)| service).collect())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// No primary interface (asleep, or between networks) copies out null; wrapping it must
        /// not release it on the way out, as `CFRelease(NULL)` stops the app.
        #[test]
        fn a_null_result_is_none() {
            assert!(Owned::new(null()).is_none());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_this_mac() {
        let list = interfaces();
        for i in &list {
            eprintln!("{:<6} {:<22} {:?} active={} address={:?} in={} out={}", i.bsd, i.name, i.kind, i.active, i.address, i.received, i.sent);
        }
        let primary = primary();
        eprintln!("primary: {primary:?}, totals: {:?}", totals(&list));
        assert!(!list.is_empty());
        assert!(list.iter().all(|i| !skipped(&i.bsd)));
        assert!(list.iter().filter(|i| i.kind == Kind::Vpn).all(|i| i.address.is_some()));
        if let Some(primary) = primary {
            assert!(list.iter().any(|i| i.bsd == primary && i.active), "{primary} is not among the active ports");
        }
        let started = Instant::now();
        for _ in 0..10 {
            interfaces();
        }
        eprintln!("a cached reading takes {:?}", started.elapsed() / 10);
    }

    #[test]
    fn names_ports() {
        assert_eq!(kind_of("IEEE80211"), Some(Kind::WiFi));
        assert_eq!(kind_of("Ethernet"), Some(Kind::Ethernet));
        assert_eq!(kind_of("Bridge"), None);
        assert_eq!(kind_of("WWAN"), Some(Kind::Other));
        assert!(skipped("lo0") && skipped("awdl0") && skipped("anpi1") && skipped("bridge0") && skipped("ap1"));
        assert!(!skipped("en0") && is_vpn("utun4") && is_vpn("ppp0") && !is_vpn("en6"));
        assert!(!routable("fe80::1".parse().unwrap()) && routable("2a00:1450::1".parse().unwrap()));
    }
}
