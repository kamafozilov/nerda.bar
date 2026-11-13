//! Clock helpers: Unix time, ISO 8601 timestamps and the local calendar.

use std::time::{SystemTime, UNIX_EPOCH};

pub fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// Days from 1970-01-01 to the given civil date (proleptic Gregorian).
pub fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Unix seconds of an ISO 8601 timestamp: `2026-11-12T10:20:30Z`, with optional fractional
/// seconds and a `+05:00` style offset in place of `Z`.
pub fn parse_iso(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() < 19 {
        return None;
    }
    let number = |from: usize, to: usize| text.get(from..to)?.parse::<i64>().ok();
    let (year, month, day) = (number(0, 4)?, number(5, 7)?, number(8, 10)?);
    let (hour, minute, second) = (number(11, 13)?, number(14, 16)?, number(17, 19)?);
    if bytes[4] != b'-' || bytes[7] != b'-' || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let mut rest = &text[19..];
    if let Some(fraction) = rest.strip_prefix('.') {
        rest = fraction.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    let offset = match rest.as_bytes().first() {
        None | Some(b'Z' | b'z') => 0,
        Some(sign @ (b'+' | b'-')) => {
            let digits: String = rest[1..].chars().filter(char::is_ascii_digit).collect();
            let hours = digits.get(0..2)?.parse::<i64>().ok()?;
            let minutes = digits.get(2..4).and_then(|m| m.parse::<i64>().ok()).unwrap_or(0);
            (hours * 3600 + minutes * 60) * if *sign == b'-' { -1 } else { 1 }
        }
        Some(_) => return None,
    };
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offset)
}

/// A moment on the local calendar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Local {
    /// Days from 1970-01-01 to the local date.
    pub day: i32,
    pub hour: u8,
    pub month: u8,
    pub day_of_month: u8,
    /// 0 is Sunday.
    pub weekday: u8,
}

/// The local calendar date and hour of a Unix time, in the system's time zone.
pub fn local(unix: i64) -> Local {
    let time: libc::time_t = unix;
    // SAFETY: `localtime_r` fills the `tm` it is given and keeps no pointer to it.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&time, &mut tm);
        tm
    };
    Local {
        day: days_from_civil(tm.tm_year as i64 + 1900, tm.tm_mon as i64 + 1, tm.tm_mday as i64) as i32,
        hour: tm.tm_hour as u8,
        month: tm.tm_mon as u8 + 1,
        day_of_month: tm.tm_mday as u8,
        weekday: tm.tm_wday as u8,
    }
}

/// The local date of a day number from [`Local::day`], read at its noon so a daylight saving
/// change can't move it.
pub fn local_day(day: i32) -> Local {
    // Noon UTC of that date is the same date in every zone within twelve hours of UTC.
    let unix = day as i64 * 86_400 + 43_200;
    let time: libc::time_t = unix;
    // SAFETY: as `local`.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::gmtime_r(&time, &mut tm);
        tm
    };
    Local { day, hour: 12, month: tm.tm_mon as u8 + 1, day_of_month: tm.tm_mday as u8, weekday: tm.tm_wday as u8 }
}

/// The local time of day: "14:05", or "14:05:30" with `seconds`.
pub fn clock(unix: i64, seconds: bool) -> String {
    let time: libc::time_t = unix;
    // SAFETY: as `local`.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&time, &mut tm);
        tm
    };
    if seconds { format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec) } else { format!("{:02}:{:02}", tm.tm_hour, tm.tm_min) }
}

/// The system time zone's offset from UTC at a Unix time, in seconds.
fn utc_offset(unix: i64) -> i64 {
    let time: libc::time_t = unix;
    // SAFETY: as `local`.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&time, &mut tm);
        tm
    };
    tm.tm_gmtoff as i64
}

/// Unix seconds of the start of `hour` on the local day `day` (as [`Local::day`]).
pub fn local_hour_start(day: i32, hour: u8) -> i64 {
    let wall = day as i64 * 86_400 + hour as i64 * 3600;
    // The offset at a first guess, then at the moment it gives, in case the two straddle a
    // daylight saving change.
    let guess = wall - utc_offset(wall);
    wall - utc_offset(guess)
}

/// A moment within the week: "Wed 14:05".
pub fn weekday_clock(unix: i64) -> String {
    format!("{} {}", WEEKDAYS[local(unix).weekday as usize], clock(unix, false))
}

pub const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
pub const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// A span as its two largest units: "2d 4h", "1h 49m", "12m", and "under a minute" as "<1m".
pub fn span(seconds: i64) -> String {
    let seconds = seconds.max(0);
    let (days, hours, minutes) = (seconds / 86_400, seconds % 86_400 / 3600, seconds % 3600 / 60);
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m")
    } else {
        "<1m".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_timestamps_parse_with_fractions_and_offsets() {
        assert_eq!(parse_iso("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso("2026-11-12T10:20:30Z"), Some(1_794_478_830));
        assert_eq!(parse_iso("2026-11-12T10:20:30.123Z"), Some(1_794_478_830));
        assert_eq!(parse_iso("2026-11-12T15:20:30.123456+05:00"), Some(1_794_478_830));
        assert_eq!(parse_iso("2026-11-12T05:20:30-0500"), Some(1_794_478_830));
        assert_eq!(parse_iso("2026-11-12T10:20:30"), Some(1_794_478_830));
        assert_eq!(parse_iso("2026-11-12"), None);
        assert_eq!(parse_iso("not a timestamp at all"), None);
    }

    #[test]
    fn civil_days_count_from_the_unix_epoch() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(days_from_civil(1969, 12, 31), -1);
    }

    #[test]
    fn a_day_number_reads_back_as_its_date() {
        let day = days_from_civil(2026, 11, 12) as i32;
        let date = local_day(day);
        assert_eq!((date.month, date.day_of_month, date.weekday), (11, 12, 4));
    }

    #[test]
    fn a_local_hour_starts_where_the_clock_says() {
        for unix in [1_794_484_800, 1_794_484_800 + 13 * 3600 + 1234, 1_774_000_000] {
            let at = local(unix);
            let start = local_hour_start(at.day, at.hour);
            assert!(start <= unix && unix - start < 3600, "{unix} is in the hour from {start}");
            assert_eq!(clock(start, true).get(3..), Some("00:00"));
        }
    }

    #[test]
    fn spans_show_their_two_largest_units() {
        assert_eq!(span(2 * 86_400 + 4 * 3600 + 59), "2d 4h");
        assert_eq!(span(3600 + 49 * 60), "1h 49m");
        assert_eq!(span(12 * 60 + 5), "12m");
        assert_eq!(span(40), "<1m");
        assert_eq!(span(-5), "<1m");
    }
}
