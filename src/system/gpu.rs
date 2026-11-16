//! The GPU, read from the `IOAccelerator` service's `PerformanceStatistics`, the numbers
//! Activity Monitor's GPU History draws: how busy it is overall and in its rendering and
//! geometry stages, and the memory its work holds.

use super::iokit;

/// The GPU at one reading.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub struct Gpu {
    pub cores: Option<u32>,
    /// From 0 to 1, as are the two below.
    pub busy: f32,
    /// The stage that shades pixels.
    pub rendering: f32,
    /// The stage that sorts geometry into tiles.
    pub geometry: f32,
    /// Bytes of memory in use by GPU work.
    pub memory: u64,
}

/// The first GPU that reports its statistics; `None` when none does.
#[allow(dead_code)]
pub fn read() -> Option<Gpu> {
    iokit::services(c"IOAccelerator").find_map(|service| {
        let properties = service.properties()?;
        let stats = properties.dict()?.dict("PerformanceStatistics")?;
        let share = |name: &str| stats.i64(name).map_or(0.0, |p| (p as f32 / 100.0).clamp(0.0, 1.0));
        let memory = stats.i64("In use system memory").or_else(|| stats.i64("Alloc system memory")).unwrap_or(0).max(0) as u64;
        // The count sits on the accelerator, or on an `AGXAccelerator` above it.
        let cores = service.search("gpu-core-count").and_then(|n| n.i64()).filter(|&n| n > 0).map(|n| n as u32);
        Some(Gpu { cores, busy: share("Device Utilization %"), rendering: share("Renderer Utilization %"), geometry: share("Tiler Utilization %"), memory })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_this_macs_gpu() {
        let Some(gpu) = read() else {
            eprintln!("gpu: none reports statistics on this Mac");
            return;
        };
        eprintln!("gpu: {gpu:#?}");
        for share in [gpu.busy, gpu.rendering, gpu.geometry] {
            assert!((0.0..=1.0).contains(&share));
        }
        assert!(gpu.cores.is_none_or(|c| c < 1024));
        assert!(gpu.memory < 1 << 40);
    }
}
