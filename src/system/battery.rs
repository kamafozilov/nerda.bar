//! The battery, read from the `AppleSmartBattery` service the way System Settings reads it:
//! the charge, whether it is plugged in and charging, how long it has left, the power it
//! gives, and how worn it is.
//!
//! On Apple silicon `CurrentCapacity` and `MaxCapacity` are percents; the raw mAh figures sit
//! in a nested `BatteryData` dictionary on recent macOS, so those are looked for in both
//! places.

use super::iokit::{self, Dict, PowerSources};

/// The battery's state at one reading.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub struct Battery {
    /// From 0 to 1.
    pub charge: f32,
    /// On a charger.
    pub plugged: bool,
    pub charging: bool,
    pub full: bool,
    /// To empty on battery, to full while charging; `None` while macOS is still working it
    /// out, or when it is neither.
    pub minutes_left: Option<u32>,
    /// Watts drawn from the battery; `None` unless it is discharging.
    pub watts: Option<f32>,
    pub cycle_count: Option<u32>,
    /// What it holds now against what it held new, from 0 to 1.
    pub health: Option<f32>,
    /// "Normal", "Service Recommended" and the like, as the battery menu says it.
    pub condition: Option<String>,
    /// Degrees Celsius.
    pub temperature: Option<f32>,
}

/// macOS's "still calculating" for a time.
const UNKNOWN_TIME: i64 = 65535;

/// The battery now; `None` on a Mac without one.
#[allow(dead_code)]
pub fn read() -> Option<Battery> {
    let service = iokit::services(c"AppleSmartBattery").next()?;
    let properties = service.properties()?;
    let top = properties.dict()?;
    if top.bool("BatteryInstalled") == Some(false) {
        return None;
    }
    let data = top.dict("BatteryData");
    // A value at the top level, else in `BatteryData`.
    let number = |name: &str| top.i64(name).or_else(|| data.and_then(|d| d.i64(name)));

    let current = number("CurrentCapacity")?;
    let max = number("MaxCapacity").filter(|&m| m > 0)?;
    let plugged = top.bool("ExternalConnected").unwrap_or(false);
    let charging = top.bool("IsCharging").unwrap_or(false);
    let full = top.bool("FullyCharged").unwrap_or(false);

    let time = |name: &str| number(name).filter(|&m| m > 0 && m < UNKNOWN_TIME).map(|m| m as u32);
    let minutes_left = if charging {
        time("AvgTimeToFull")
    } else if !plugged {
        time("AvgTimeToEmpty").or_else(|| time("TimeRemaining"))
    } else {
        None
    };

    let amperage = number("InstantAmperage").filter(|&a| a != 0).or_else(|| number("Amperage")).map(signed);
    let watts = match (number("Voltage"), amperage) {
        (Some(millivolts), Some(milliamps)) if milliamps < 0 => Some((millivolts as f64 * -milliamps as f64 / 1e6) as f32),
        _ => None,
    };

    let design = number("DesignCapacity").filter(|&d| d > 0);
    // The nominal capacity is what System Settings' Maximum Capacity follows; the raw maximum
    // swings with temperature and the last charge.
    let now = number("NominalChargeCapacity").or_else(|| number("AppleRawMaxCapacity")).filter(|&n| n > 0);
    let health = design.zip(now).map(|(design, now)| (now as f32 / design as f32).min(1.0));

    Some(Battery {
        charge: (current as f32 / max as f32).clamp(0.0, 1.0),
        plugged,
        charging,
        full,
        minutes_left,
        watts,
        cycle_count: number("CycleCount").filter(|&c| c >= 0).map(|c| c as u32),
        health,
        // Where macOS doesn't say, the battery is due for service below 80% of its capacity new,
        // as Apple's own guidance has it.
        condition: condition().or_else(|| health.map(|h| if h >= 0.8 { "Normal" } else { "Service Recommended" }.to_owned())),
        temperature: temperature(&number),
    })
}

/// Degrees Celsius, from hundredths of a degree. Older macOS keeps it beside the other
/// values; recent macOS keeps it only in the `BatteryData` of the `AppleSmartBatteryPack`
/// below, which is read only then.
fn temperature(number: &dyn Fn(&str) -> Option<i64>) -> Option<f32> {
    let hundredths = |n: Option<i64>| n.filter(|&t| t > 0).map(|t| t as f32 / 100.0);
    if let Some(celsius) = hundredths(number("Temperature").or_else(|| number("VirtualTemperature"))) {
        return Some(celsius);
    }
    let pack = iokit::services(c"AppleSmartBatteryPack").next()?.properties()?;
    let data = pack.dict()?.dict("BatteryData")?;
    hundredths(data.i64("Temperature").or_else(|| data.i64("VirtualTemperature")))
}

/// A current some drivers keep as a 32-bit two's complement inside the 64-bit number; a
/// value above 2^31 that fits in 32 bits is a negative one.
fn signed(value: i64) -> i64 {
    if (0x8000_0000..=0xFFFF_FFFF).contains(&value) { value as u32 as i32 as i64 } else { value }
}

/// The internal battery's condition from the power-source list, in System Settings' words.
fn condition() -> Option<String> {
    let sources = PowerSources::copy()?;
    let battery: Dict<'_> = sources.descriptions().find(|d| d.string("Type").as_deref() == Some("InternalBattery"))?;
    if let Some(condition) = battery.string("BatteryHealthCondition").filter(|c| !c.is_empty()) {
        return Some(condition);
    }
    battery.string("BatteryHealth").map(|health| match health.as_str() {
        // The power-source list says "Good" where System Settings says "Normal".
        "Good" => "Normal".to_owned(),
        _ => health,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_reads_32_bit_negatives() {
        assert_eq!(signed(0xFFFF_FC18), -1000);
        assert_eq!(signed(-1000), -1000);
        assert_eq!(signed(1500), 1500);
    }

    #[test]
    fn reads_this_macs_battery() {
        let Some(battery) = read() else {
            eprintln!("battery: none on this Mac");
            return;
        };
        eprintln!("battery: {battery:#?}");
        assert!((0.0..=1.0).contains(&battery.charge));
        assert!(battery.health.is_none_or(|h| h > 0.0 && h <= 1.0));
        assert!(battery.watts.is_none_or(|w| w > 0.0 && w < 300.0));
        assert!(battery.temperature.is_none_or(|t| t > -20.0 && t < 90.0));
        assert!(battery.minutes_left.is_none_or(|m| m < 65535));
        assert!(!(battery.charging && !battery.plugged));
    }
}
