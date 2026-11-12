//! The global allocator: the system's, except that large blocks are mapped straight from the
//! kernel and unmapped when freed.
//!
//! A scan reads session logs tens of megabytes at a time. macOS's malloc keeps freed blocks
//! of that size in a cache that still counts as the app's memory, so one scan would leave the
//! footprint that much higher for good. A mapping goes back to the system the moment it's
//! freed.

use std::alloc::{GlobalAlloc, Layout, System};

/// Blocks this size and up are mapped. Below it the system allocator is faster and its
/// caches are small.
const LARGE: usize = 1 << 20;
/// Mappings are page aligned, which covers any alignment up to a page.
const PAGE: usize = 16 << 10;

pub struct Returning;

fn mapped(layout: Layout) -> bool {
    layout.size() >= LARGE && layout.align() <= PAGE
}

// SAFETY: every block is freed by the allocator that made it: `mapped` depends only on the
// layout, which `dealloc` and `realloc` receive unchanged.
unsafe impl GlobalAlloc for Returning {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if !mapped(layout) {
            // SAFETY: forwarded with the caller's layout.
            return unsafe { System.alloc(layout) };
        }
        // SAFETY: an anonymous private mapping of `size` bytes; no file or address is involved.
        let block = unsafe { libc::mmap(std::ptr::null_mut(), layout.size(), libc::PROT_READ | libc::PROT_WRITE, libc::MAP_PRIVATE | libc::MAP_ANON, -1, 0) };
        if block == libc::MAP_FAILED { std::ptr::null_mut() } else { block.cast() }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if mapped(layout) {
            // A fresh anonymous mapping is already zero.
            // SAFETY: as `alloc`.
            unsafe { self.alloc(layout) }
        } else {
            // SAFETY: forwarded with the caller's layout.
            unsafe { System.alloc_zeroed(layout) }
        }
    }

    unsafe fn dealloc(&self, block: *mut u8, layout: Layout) {
        if mapped(layout) {
            // SAFETY: `block` came from `mmap` with this size.
            unsafe { libc::munmap(block.cast(), layout.size()) };
        } else {
            // SAFETY: `block` came from `System` with this layout.
            unsafe { System.dealloc(block, layout) };
        }
    }

    unsafe fn realloc(&self, block: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_layout = Layout::from_size_align(new_size, layout.align());
        if !mapped(layout) && new_layout.as_ref().is_ok_and(|l| !mapped(*l)) {
            // SAFETY: both sides belong to `System`.
            return unsafe { System.realloc(block, layout, new_size) };
        }
        // Crossing the threshold, or growing a mapping: move to a new block.
        let Ok(new_layout) = new_layout else { return std::ptr::null_mut() };
        // SAFETY: a valid layout; the copy stays within both blocks, which don't overlap.
        unsafe {
            let new_block = self.alloc(new_layout);
            if !new_block.is_null() {
                std::ptr::copy_nonoverlapping(block, new_block, layout.size().min(new_size));
                self.dealloc(block, layout);
            }
            new_block
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn large_and_small_blocks_grow_shrink_and_keep_their_contents() {
        let mut bytes: Vec<u8> = Vec::new();
        for i in 0..(3 << 20) {
            bytes.push((i % 251) as u8);
        }
        assert!(bytes.iter().enumerate().all(|(i, b)| *b == (i % 251) as u8));
        bytes.truncate(1000);
        bytes.shrink_to_fit();
        assert!(bytes.iter().enumerate().all(|(i, b)| *b == (i % 251) as u8));
        let zeroed = vec![0u8; 2 << 20];
        assert!(zeroed.iter().all(|b| *b == 0));
        let pixels = vec![7u32; 1 << 20];
        assert_eq!(pixels.iter().map(|p| u64::from(*p)).sum::<u64>(), 7 << 20);
    }
}
