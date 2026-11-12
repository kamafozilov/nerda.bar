//! The kernel's own counters, read the way Activity Monitor reads them: per-core CPU ticks from
//! `host_processor_info`, page counts from `host_statistics64`, and the rest from `sysctl`.
//! None of it needs a privilege.

// libc marks its Mach calls deprecated in favour of the `mach2` crate; these few don't warrant
// another dependency.
#![allow(deprecated)]

use std::ffi::CStr;
use std::mem::{size_of, zeroed};

unsafe extern "C" {
    fn mach_port_deallocate(task: libc::mach_port_t, name: libc::mach_port_t) -> libc::kern_return_t;
}

/// The host port, asked for once: every `mach_host_self` call adds a reference to it.
pub struct Host(libc::mach_port_t);

impl Host {
    pub fn new() -> Host {
        // SAFETY: always valid; returns a send right to this host.
        Host(unsafe { libc::mach_host_self() })
    }

    /// Each core's cumulative ticks: user, system, idle and nice.
    pub fn cpu_ticks(&self) -> Option<Vec<[u32; 4]>> {
        let mut cpus: libc::natural_t = 0;
        let mut info: libc::processor_info_array_t = std::ptr::null_mut();
        let mut count: libc::mach_msg_type_number_t = 0;
        // SAFETY: the kernel allocates `info` with `count` integers in it, which are read and
        // then given back.
        unsafe {
            if libc::host_processor_info(self.0, libc::PROCESSOR_CPU_LOAD_INFO, &mut cpus, &mut info, &mut count) != libc::KERN_SUCCESS || info.is_null() {
                return None;
            }
            let ints = std::slice::from_raw_parts(info as *const u32, count as usize);
            let state = libc::CPU_STATE_MAX as usize;
            let ticks = ints.chunks_exact(state).take(cpus as usize).map(|c| [c[0], c[1], c[2], c[3]]).collect();
            libc::vm_deallocate(libc::mach_task_self(), info as libc::vm_address_t, count as usize * size_of::<libc::integer_t>());
            Some(ticks)
        }
    }

    pub fn vm_statistics(&self) -> Option<libc::vm_statistics64> {
        // SAFETY: the kernel fills at most `count` integers of the zeroed struct.
        unsafe {
            let mut stats: libc::vm_statistics64 = zeroed();
            let mut count = (size_of::<libc::vm_statistics64>() / size_of::<libc::integer_t>()) as libc::mach_msg_type_number_t;
            let ok = libc::host_statistics64(self.0, libc::HOST_VM_INFO64, &mut stats as *mut _ as libc::host_info64_t, &mut count);
            (ok == libc::KERN_SUCCESS).then_some(stats)
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        // SAFETY: gives back the right `mach_host_self` added.
        unsafe { mach_port_deallocate(libc::mach_task_self(), self.0) };
    }
}

/// A fixed-size `sysctl` value.
pub fn sysctl<T: Copy>(name: &CStr) -> Option<T> {
    // SAFETY: `sysctlbyname` writes at most `size` bytes into the zeroed value.
    unsafe {
        let mut value: T = zeroed();
        let mut size = size_of::<T>();
        let ok = libc::sysctlbyname(name.as_ptr(), &mut value as *mut T as *mut libc::c_void, &mut size, std::ptr::null_mut(), 0) == 0;
        (ok && size == size_of::<T>()).then_some(value)
    }
}

/// A string `sysctl` value.
pub fn sysctl_string(name: &CStr) -> Option<String> {
    // SAFETY: asks for the size first, then writes at most that many bytes.
    unsafe {
        let mut size = 0usize;
        if libc::sysctlbyname(name.as_ptr(), std::ptr::null_mut(), &mut size, std::ptr::null_mut(), 0) != 0 || size == 0 {
            return None;
        }
        let mut buffer = vec![0u8; size];
        if libc::sysctlbyname(name.as_ptr(), buffer.as_mut_ptr() as *mut libc::c_void, &mut size, std::ptr::null_mut(), 0) != 0 {
            return None;
        }
        buffer.truncate(size);
        let text = CStr::from_bytes_until_nul(&buffer).ok()?.to_str().ok()?.trim().to_owned();
        (!text.is_empty()).then_some(text)
    }
}

/// The 1, 5 and 15-minute load averages.
pub fn load_average() -> [f64; 3] {
    let mut load = [0.0f64; 3];
    // SAFETY: writes at most 3 doubles.
    unsafe { libc::getloadavg(load.as_mut_ptr(), 3) };
    load
}

/// When the Mac last started, in Unix seconds.
pub fn boot_time() -> Option<i64> {
    sysctl::<libc::timeval>(c"kern.boottime").map(|tv| tv.tv_sec)
}
