//! The little of IOKit and CoreFoundation the readers need: finding the services of a class,
//! copying out their properties, and reading values from the dictionaries that come back.
//!
//! The readers run every few seconds for days, so every CoreFoundation object made here is
//! held by an [`Owned`] that releases it, and every IOKit object by a [`Service`] or the
//! iterator that hands them out.

use std::ffi::{CStr, c_char, c_void};
use std::marker::PhantomData;

type CFTypeRef = *const c_void;
type CFIndex = isize;
type IoObject = libc::mach_port_t;

const MAIN_PORT_DEFAULT: libc::mach_port_t = 0;
const UTF8: u32 = 0x0800_0100;
const SINT64: CFIndex = 4;
#[allow(dead_code)]
const FLOAT64: CFIndex = 6;
/// Search an entry and then its parents.
const ITERATE_RECURSIVELY_PARENTS: u32 = 1 | 2;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: CFTypeRef);
    fn CFGetTypeID(cf: CFTypeRef) -> usize;
    fn CFDictionaryGetTypeID() -> usize;
    fn CFNumberGetTypeID() -> usize;
    fn CFBooleanGetTypeID() -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFArrayGetTypeID() -> usize;
    fn CFDictionaryGetValue(dict: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
    fn CFNumberGetValue(number: CFTypeRef, kind: CFIndex, value: *mut c_void) -> bool;
    fn CFBooleanGetValue(boolean: CFTypeRef) -> bool;
    fn CFStringCreateWithBytes(alloc: CFTypeRef, bytes: *const u8, len: CFIndex, encoding: u32, external: bool) -> CFTypeRef;
    fn CFStringGetLength(string: CFTypeRef) -> CFIndex;
    fn CFStringGetMaximumSizeForEncoding(len: CFIndex, encoding: u32) -> CFIndex;
    fn CFStringGetCString(string: CFTypeRef, buffer: *mut c_char, size: CFIndex, encoding: u32) -> bool;
    fn CFArrayGetCount(array: CFTypeRef) -> CFIndex;
    fn CFArrayGetValueAtIndex(array: CFTypeRef, index: CFIndex) -> CFTypeRef;
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOServiceMatching(name: *const c_char) -> CFTypeRef;
    fn IOServiceGetMatchingServices(port: libc::mach_port_t, matching: CFTypeRef, iterator: *mut IoObject) -> libc::kern_return_t;
    fn IOIteratorNext(iterator: IoObject) -> IoObject;
    fn IOObjectRelease(object: IoObject) -> libc::kern_return_t;
    fn IORegistryEntryCreateCFProperties(entry: IoObject, properties: *mut CFTypeRef, alloc: CFTypeRef, options: u32) -> libc::kern_return_t;
    fn IORegistryEntrySearchCFProperty(entry: IoObject, plane: *const c_char, key: CFTypeRef, alloc: CFTypeRef, options: u32) -> CFTypeRef;
    fn IOPSCopyPowerSourcesInfo() -> CFTypeRef;
    fn IOPSCopyPowerSourcesList(blob: CFTypeRef) -> CFTypeRef;
    fn IOPSGetPowerSourceDescription(blob: CFTypeRef, source: CFTypeRef) -> CFTypeRef;
}

/// A CoreFoundation object made by a `Create` or `Copy` call, released when dropped.
pub(super) struct Owned(CFTypeRef);

impl Owned {
    /// Takes ownership of what a `Create` or `Copy` call returned; `None` for null.
    fn new(raw: CFTypeRef) -> Option<Owned> {
        (!raw.is_null()).then_some(Owned(raw))
    }

    /// The object as a dictionary, if it is one.
    pub(super) fn dict(&self) -> Option<Dict<'_>> {
        Dict::borrow(self.0)
    }

    /// The object as a number, if it is one.
    pub(super) fn i64(&self) -> Option<i64> {
        number_i64(self.0)
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: `self.0` is non-null and this code holds the one reference it was given.
        unsafe { CFRelease(self.0) };
    }
}

/// A CFString key, made for one lookup and released after it.
fn key(name: &str) -> Option<Owned> {
    // SAFETY: the bytes are valid UTF-8 for their length; CoreFoundation copies them.
    Owned::new(unsafe { CFStringCreateWithBytes(std::ptr::null(), name.as_ptr(), name.len() as CFIndex, UTF8, false) })
}

fn is(value: CFTypeRef, type_id: unsafe extern "C" fn() -> usize) -> bool {
    // SAFETY: `value` is a live, non-null CoreFoundation object; the type-ID calls are pure.
    !value.is_null() && unsafe { CFGetTypeID(value) == type_id() }
}

fn number_i64(value: CFTypeRef) -> Option<i64> {
    if !is(value, CFNumberGetTypeID) {
        return None;
    }
    let mut out = 0i64;
    // SAFETY: `value` is a CFNumber and `out` has room for a 64-bit integer. A false return
    // only means the value was rounded (a float), which is fine here.
    unsafe { CFNumberGetValue(value, SINT64, &mut out as *mut i64 as *mut c_void) };
    Some(out)
}

/// A dictionary owned by something else (an [`Owned`] or a dictionary inside one), valid
/// for as long as that owner lives.
#[derive(Clone, Copy)]
pub(super) struct Dict<'a> {
    raw: CFTypeRef,
    _owner: PhantomData<&'a Owned>,
}

impl<'a> Dict<'a> {
    fn borrow(raw: CFTypeRef) -> Option<Dict<'a>> {
        is(raw, CFDictionaryGetTypeID).then_some(Dict { raw, _owner: PhantomData })
    }

    /// The value under `name`, borrowed from this dictionary.
    fn get(&self, name: &str) -> CFTypeRef {
        let Some(key) = key(name) else { return std::ptr::null() };
        // SAFETY: `self.raw` is a live CFDictionary and `key` a live CFString; the value
        // returned is not retained, and stays alive with the dictionary.
        unsafe { CFDictionaryGetValue(self.raw, key.0) }
    }

    /// An integer, with the 64 bits as stored: IOKit keeps signed values such as a current
    /// as unsigned 64-bit numbers, which read back here as the negative numbers they are.
    pub(super) fn i64(&self, name: &str) -> Option<i64> {
        number_i64(self.get(name))
    }

    #[allow(dead_code)]
    pub(super) fn f64(&self, name: &str) -> Option<f64> {
        let value = self.get(name);
        if !is(value, CFNumberGetTypeID) {
            return None;
        }
        let mut out = 0f64;
        // SAFETY: `value` is a CFNumber and `out` has room for a double.
        unsafe { CFNumberGetValue(value, FLOAT64, &mut out as *mut f64 as *mut c_void) };
        Some(out)
    }

    /// A boolean, or a number read as one: some drivers publish flags as 0 and 1.
    pub(super) fn bool(&self, name: &str) -> Option<bool> {
        let value = self.get(name);
        if is(value, CFBooleanGetTypeID) {
            // SAFETY: `value` is a CFBoolean.
            return Some(unsafe { CFBooleanGetValue(value) });
        }
        number_i64(value).map(|n| n != 0)
    }

    pub(super) fn string(&self, name: &str) -> Option<String> {
        let value = self.get(name);
        if !is(value, CFStringGetTypeID) {
            return None;
        }
        // SAFETY: `value` is a CFString; the buffer is sized for its longest UTF-8 form plus
        // the terminating nul, and CoreFoundation writes at most that many bytes.
        unsafe {
            let size = CFStringGetMaximumSizeForEncoding(CFStringGetLength(value), UTF8) + 1;
            let mut buffer = vec![0u8; size.max(1) as usize];
            if !CFStringGetCString(value, buffer.as_mut_ptr() as *mut c_char, buffer.len() as CFIndex, UTF8) {
                return None;
            }
            CStr::from_bytes_until_nul(&buffer).ok()?.to_str().ok().map(str::to_owned)
        }
    }

    /// A dictionary nested under `name`, living as long as this one.
    pub(super) fn dict(&self, name: &str) -> Option<Dict<'a>> {
        Dict::borrow(self.get(name))
    }
}

/// One IOKit service, released when dropped.
pub(super) struct Service(IoObject);

impl Service {
    /// A copy of all of the service's properties, as a dictionary.
    pub(super) fn properties(&self) -> Option<Owned> {
        let mut properties: CFTypeRef = std::ptr::null();
        // SAFETY: `self.0` is a live registry entry; on success the copy is ours to release.
        let ok = unsafe { IORegistryEntryCreateCFProperties(self.0, &mut properties, std::ptr::null(), 0) };
        if ok != libc::KERN_SUCCESS {
            return None;
        }
        Owned::new(properties)
    }

    /// One property, looked for on the service and then on each of its parents in turn.
    pub(super) fn search(&self, name: &str) -> Option<Owned> {
        let key = key(name)?;
        // SAFETY: `self.0` is a live registry entry, `key` a live CFString and the plane name
        // a nul-terminated string; the value returned is a copy for us to release.
        Owned::new(unsafe { IORegistryEntrySearchCFProperty(self.0, c"IOService".as_ptr(), key.0, std::ptr::null(), ITERATE_RECURSIVELY_PARENTS) })
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        // SAFETY: gives back the reference `IOIteratorNext` handed over.
        unsafe { IOObjectRelease(self.0) };
    }
}

/// The services registered under an IOKit class (or a subclass of it), one at a time.
pub(super) struct Services(IoObject);

impl Iterator for Services {
    type Item = Service;

    fn next(&mut self) -> Option<Service> {
        // SAFETY: `self.0` is a live iterator; a non-zero object is a reference we now hold.
        let object = unsafe { IOIteratorNext(self.0) };
        (object != 0).then_some(Service(object))
    }
}

impl Drop for Services {
    fn drop(&mut self) {
        // SAFETY: gives back the iterator `IOServiceGetMatchingServices` made.
        unsafe { IOObjectRelease(self.0) };
    }
}

/// The services of `class`, such as `AppleSmartBattery`; empty when there are none.
pub(super) fn services(class: &CStr) -> Services {
    let mut iterator: IoObject = 0;
    // SAFETY: `class` is nul-terminated. `IOServiceGetMatchingServices` takes the matching
    // dictionary's one reference whether it succeeds or not, so it is not released here.
    unsafe {
        let matching = IOServiceMatching(class.as_ptr());
        if matching.is_null() || IOServiceGetMatchingServices(MAIN_PORT_DEFAULT, matching, &mut iterator) != libc::KERN_SUCCESS {
            iterator = 0;
        }
    }
    Services(iterator)
}

/// The power sources macOS reports (the internal battery, a UPS), as the battery menu sees
/// them.
pub(super) struct PowerSources {
    blob: Owned,
    list: Owned,
}

impl PowerSources {
    pub(super) fn copy() -> Option<PowerSources> {
        // SAFETY: both calls return copies for us to release, or null.
        let blob = Owned::new(unsafe { IOPSCopyPowerSourcesInfo() })?;
        let list = Owned::new(unsafe { IOPSCopyPowerSourcesList(blob.0) })?;
        is(list.0, CFArrayGetTypeID).then_some(PowerSources { blob, list })
    }

    /// Each source's description, living as long as these sources.
    pub(super) fn descriptions(&self) -> impl Iterator<Item = Dict<'_>> {
        // SAFETY: `self.list` is a live CFArray.
        let count = unsafe { CFArrayGetCount(self.list.0) };
        (0..count).filter_map(move |index| {
            // SAFETY: `index` is in bounds; the item and its description are not retained and
            // live as long as the list and the blob they came from.
            let description = unsafe { IOPSGetPowerSourceDescription(self.blob.0, CFArrayGetValueAtIndex(self.list.0, index)) };
            Dict::borrow(description)
        })
    }
}
