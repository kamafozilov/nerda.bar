//! Readings kept over time at three resolutions, each a fixed number of buckets on the wall
//! clock: 5 minutes in 2-second buckets, an hour in 20-second ones and a day in 5-minute ones.
//!
//! A reading covers a span of time, not an instant: the CPU's share is an average over the time
//! since the last reading, so a reading taken every 20 seconds still fills each 2-second bucket
//! it spans, and a day's chart is exact whatever the sampling rate was. Each bucket keeps the
//! average and the highest reading in it, so a short spike still shows in a 5-minute bucket.

use std::collections::VecDeque;

/// One resolution: how long a bucket is and how many are kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tier {
    pub label: &'static str,
    /// Seconds per bucket.
    pub step: i64,
    pub len: usize,
}

pub const TIERS: [Tier; 3] = [
    Tier { label: "5m", step: 2, len: 150 },
    Tier { label: "1h", step: 20, len: 180 },
    Tier { label: "24h", step: 300, len: 288 },
];

/// A bucket as a chart reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub avg: f32,
    pub max: f32,
}

#[derive(Clone, Copy, Debug)]
struct Bucket {
    /// The bucket's start in Unix seconds, divided by its tier's step.
    at: i64,
    sum: f64,
    weight: f64,
    max: f32,
}

/// One measure (the CPU's share in use, say) at every resolution.
#[derive(Clone, Debug, Default)]
pub struct Series {
    tiers: [VecDeque<Bucket>; 3],
}

impl Series {
    /// Records `value` as holding from `from` to `to` (Unix seconds, `from` < `to`).
    pub fn add(&mut self, from: f64, to: f64, value: f32) {
        if to.partial_cmp(&from) != Some(std::cmp::Ordering::Greater) || !value.is_finite() {
            return;
        }
        for (tier, buckets) in TIERS.iter().zip(&mut self.tiers) {
            let step = tier.step as f64;
            let last = (to / step).floor() as i64;
            // Never more buckets than the tier keeps, however long the span.
            let first = ((from / step).floor() as i64).max(last - tier.len as i64 + 1);
            for at in first..=last {
                let start = at as f64 * step;
                let overlap = (to.min(start + step) - from.max(start)).max(0.0);
                if overlap <= 0.0 {
                    continue;
                }
                match buckets.back_mut() {
                    Some(bucket) if bucket.at == at => {
                        bucket.sum += value as f64 * overlap;
                        bucket.weight += overlap;
                        bucket.max = bucket.max.max(value);
                    }
                    Some(bucket) if bucket.at > at => {}
                    _ => buckets.push_back(Bucket { at, sum: value as f64 * overlap, weight: overlap, max: value }),
                }
            }
            while buckets.front().is_some_and(|bucket| bucket.at <= last - tier.len as i64) {
                buckets.pop_front();
            }
        }
    }

    /// The tier's buckets up to the one holding `now`, oldest first, `None` where nothing was
    /// recorded (the Mac slept, or the app wasn't running).
    pub fn points(&self, tier: usize, now: f64) -> Vec<Option<Point>> {
        let Tier { step, len, .. } = TIERS[tier];
        let last = (now / step as f64).floor() as i64;
        let first = last - len as i64 + 1;
        let mut out = vec![None; len];
        for bucket in &self.tiers[tier] {
            if (first..=last).contains(&bucket.at) && bucket.weight > 0.0 {
                out[(bucket.at - first) as usize] = Some(Point { avg: (bucket.sum / bucket.weight) as f32, max: bucket.max });
            }
        }
        out
    }
}

/// The Unix second the bucket at `index` of `points(tier, now)` starts at.
pub fn bucket_start(tier: usize, now: f64, index: usize) -> i64 {
    let Tier { step, len, .. } = TIERS[tier];
    ((now / step as f64).floor() as i64 - len as i64 + 1 + index as i64) * step
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_reading_fills_every_bucket_it_spans() {
        let mut series = Series::default();
        series.add(1000.0, 1020.0, 0.5);
        let points = series.points(0, 1020.0);
        // 1000 to 1020 is ten 2-second buckets, and the bucket holding 1020 is still empty.
        let filled: Vec<f32> = points.iter().flatten().map(|p| p.avg).collect();
        assert_eq!(filled, vec![0.5; 10]);
        assert!(points[points.len() - 1].is_none());
    }

    #[test]
    fn a_bucket_averages_by_time_and_keeps_its_peak() {
        let mut series = Series::default();
        series.add(0.0, 200.0, 0.1);
        series.add(200.0, 300.0, 0.7);
        let day = series.points(2, 299.0);
        let point = day[day.len() - 1].unwrap();
        assert!((point.avg - 0.3).abs() < 1e-6, "{}", point.avg);
        assert_eq!(point.max, 0.7);
    }

    #[test]
    fn old_buckets_fall_away() {
        let mut series = Series::default();
        series.add(0.0, 2.0, 1.0);
        series.add(1000.0, 1002.0, 0.2);
        assert_eq!(series.tiers[0].len(), 1, "the 5-minute tier forgot the first reading");
        assert_eq!(series.tiers[2].len(), 2, "the day kept both");
    }

    #[test]
    fn a_span_longer_than_a_tier_writes_only_what_it_keeps() {
        let mut series = Series::default();
        series.add(0.0, 10_000_001.0, 0.4);
        assert_eq!(series.tiers[0].len(), TIERS[0].len);
    }
}
