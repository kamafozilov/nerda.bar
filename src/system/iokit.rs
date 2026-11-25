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
    fn CFDictionaryGetCount(dict: CFTypeRef) -> CFIndex;
    fn CFDictionaryGetKeysAndValues(dict: CFTypeRef, keys: *mut CFTypeRef, values: *mut CFTypeRef);
    fn CFDateGetTypeID() -> usize;
    fn CFDateGetAbsoluteTime(date: CFTypeRef) -> f64;
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
    fn IOPSGetProvidingPowerSourceType(blob: CFTypeRef) -> CFTypeRef;
    fn IOPMAssertionCreateWithDescription(
        kind: CFTypeRef,
        name: CFTypeRef,
        details: CFTypeRef,
        reason: CFTypeRef,
        bundle: CFTypeRef,
        timeout: f64,
        timeout_action: CFTypeRef,
        id: *mut u32,
    ) -> libc::kern_return_t;
    fn IOPMAssertionRelease(id: u32) -> libc::kern_return_t;
    fn IOPMCopyAssertionsByProcess(by_process: *mut CFTypeRef) -> libc::kern_return_t;
}

/// A CoreFoundation object made by a `Create` or `Copy` call, released when dropped.
pub(super) struct Owned(CFTypeRef);

impl Owned {
    /// Takes ownership of what a `Create` or `Copy` call returned; `None` for null.
    fn new(raw: CFTypeRef) -> Option<Owned> {
        // `then`, not `then_some`: an `Owned` built from null would be dropped, and release it.
        (!raw.is_null()).then(|| Owned(raw))
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
        string(self.get(name))
    }

    /// A dictionary nested under `name`, living as long as this one.
    pub(super) fn dict(&self, name: &str) -> Option<Dict<'a>> {
        Dict::borrow(self.get(name))
    }
}

impl<'a> Dict<'a> {
    /// A date, as Unix seconds.
    pub(super) fn date(&self, name: &str) -> Option<f64> {
        let value = self.get(name);
        // SAFETY: `value` is a CFDate.
        is(value, CFDateGetTypeID).then(|| unsafe { CFDateGetAbsoluteTime(value) } + SINCE_2001)
    }

    /// The entries keyed by a number (the assertions, keyed by pid), each with the
    /// dictionaries in the array it holds.
    pub(super) fn arrays_by_number(&self) -> Vec<(i64, Vec<Dict<'a>>)> {
        // SAFETY: `self.raw` is a live CFDictionary.
        let count = unsafe { CFDictionaryGetCount(self.raw) }.max(0) as usize;
        let (mut keys, mut values) = (vec![std::ptr::null(); count], vec![std::ptr::null(); count]);
        // SAFETY: both buffers hold `count` entries; the keys and values are not retained and
        // live as long as the dictionary.
        unsafe { CFDictionaryGetKeysAndValues(self.raw, keys.as_mut_ptr(), values.as_mut_ptr()) };
        keys.into_iter()
            .zip(values)
            .filter_map(|(key, value)| {
                let number = number_i64(key)?;
                is(value, CFArrayGetTypeID).then(|| {
                    // SAFETY: `value` is a live CFArray; its items live as long as it does.
                    let len = unsafe { CFArrayGetCount(value) };
                    (number, (0..len).filter_map(|at| Dict::borrow(unsafe { CFArrayGetValueAtIndex(value, at) })).collect())
                })
            })
            .collect()
    }
}

/// A CFString's text, borrowed from whoever owns it.
fn string(value: CFTypeRef) -> Option<String> {
    if !is(value, CFStringGetTypeID) {
        return None;
    }
    // SAFETY: `value` is a CFString; the buffer is sized for its longest UTF-8 form plus the
    // terminating nul, and CoreFoundation writes at most that many bytes.
    unsafe {
        let size = CFStringGetMaximumSizeForEncoding(CFStringGetLength(value), UTF8) + 1;
        let mut buffer = vec![0u8; size.max(1) as usize];
        if !CFStringGetCString(value, buffer.as_mut_ptr() as *mut c_char, buffer.len() as CFIndex, UTF8) {
            return None;
        }
        CStr::from_bytes_until_nul(&buffer).ok()?.to_str().ok().map(str::to_owned)
    }
}

/// CoreFoundation counts dates from 2001; Unix from 1970.
const SINCE_2001: f64 = 978_307_200.0;

/// Holds an assertion of `kind` (`PreventUserIdleDisplaySleep`, `PreventUserIdleSystemSleep`)
/// that macOS lets go of by itself after `timeout` seconds, or never with 0. `None` when it
/// was refused.
pub(super) fn assert_awake(kind: &str, name: &str, reason: &str, timeout: f64) -> Option<u32> {
    let (kind, name, reason, release) = (key(kind)?, key(name)?, key(reason)?, key("TimeoutActionRelease")?);
    let mut id = 0u32;
    // SAFETY: every string is a live CFString; `id` receives the assertion's id.
    let status = unsafe { IOPMAssertionCreateWithDescription(kind.0, name.0, std::ptr::null(), reason.0, std::ptr::null(), timeout.max(0.0), release.0, &mut id) };
    (status == libc::KERN_SUCCESS && id != 0).then_some(id)
}

/// Lets go of an assertion [`assert_awake`] made. Releasing one that already timed out is
/// harmless.
pub(super) fn release_awake(id: u32) {
    // SAFETY: releasing an id is valid whether or not it is still held.
    unsafe { IOPMAssertionRelease(id) };
}

/// Every process's assertions, keyed by pid.
pub(super) fn assertions_by_process() -> Option<Owned> {
    let mut out: CFTypeRef = std::ptr::null();
    // SAFETY: on success `out` is a copy for us to release.
    let status = unsafe { IOPMCopyAssertionsByProcess(&mut out) };
    if status != libc::KERN_SUCCESS {
        return None;
    }
    Owned::new(out)
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
        // Lazily, so that the end of the list builds no `Service` to release.
        (object != 0).then(|| Service(object))
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

    /// Where the power comes from now: "AC Power", "Battery Power" or "UPS Power".
    pub(super) fn providing(&self) -> Option<String> {
        // SAFETY: `self.blob` is live; the string returned is not retained and lives with it.
        let kind = unsafe { IOPSGetProvidingPowerSourceType(self.blob.0) };
        string(kind)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A null result owns nothing: wrapping it must not release it on the way out, as
    /// `CFRelease(NULL)` stops the app.
    #[test]
    fn a_null_result_is_none() {
        assert!(Owned::new(std::ptr::null()).is_none());
    }
}
