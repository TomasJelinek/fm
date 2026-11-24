//! The speed chart's vertical scale: steady while a job copies.
//!
//! The scale rises at once when a sample would run above the chart's top, to the next
//! round value at least `HEADROOM` times that sample. It comes down only after the
//! samples stayed below `LOW_SHARE` of it for `LOW_MILLIS` of activity, and then by
//! one round step, if the slower speed still fits below that step with its headroom.
//! Round values are 1, 2 and 5 times a power of ten of the unit that labels the scale
//! (B, KiB, MiB, GiB and TiB per second, or items per second). Time is measured by the
//! samples' own intervals, so it does not depend on how often progress is reported.
//! Idle intervals (pauses, waits, long syncs) are neither low nor high: they leave the
//! scale and its countdown as they are.
use midnight_core::{NumberLocale, ThroughputSample};

/// A new top leaves this much room above the sample that raised it.
const HEADROOM: f64 = 1.25;
/// Below this share of the top, the samples count toward a step down.
const LOW_SHARE: f64 = 0.4;
/// Activity below `LOW_SHARE` that steps the scale down by one round value.
const LOW_MILLIS: u64 = 10_000;
const MULTIPLES: [f64; 9] = [1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0];
const BYTE_UNITS: [(&str, f64); 5] = [
    ("B", 1.0),
    ("KiB", 1024.0),
    ("MiB", 1024.0 * 1024.0),
    ("GiB", 1024.0 * 1024.0 * 1024.0),
    ("TiB", 1024.0 * 1024.0 * 1024.0 * 1024.0),
];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Scale {
    /// The chart's top in rate units; zero until the first active sample.
    top: f64,
    /// The end of the newest sample taken into account.
    seen: Option<u64>,
    /// Activity since the samples last reached `LOW_SHARE` of the top, and its peak.
    low_millis: u64,
    low_peak: f64,
}

impl Scale {
    /// Take the samples that ended after the last call into account. `level` is the
    /// highest rate a sample draws; `items` selects the item ladder over the byte one.
    pub(super) fn observe(
        &mut self,
        samples: &[ThroughputSample],
        items: bool,
        level: impl Fn(&ThroughputSample) -> f64,
    ) {
        for sample in samples {
            if self.seen.is_some_and(|seen| sample.elapsed_millis <= seen) {
                continue;
            }
            // The newest interval can grow between two reports: only its new part counts.
            let from = sample.start_millis.max(self.seen.unwrap_or(0));
            let duration = sample.elapsed_millis.saturating_sub(from);
            self.seen = Some(sample.elapsed_millis);
            let level = level(sample);
            if !(level.is_finite() && level > 0.0) {
                continue;
            }
            if self.top <= 0.0 || level > self.top {
                self.top = round_up(level * HEADROOM, items);
                self.reset_low();
            } else if level < self.top * LOW_SHARE {
                self.low_millis = self.low_millis.saturating_add(duration);
                self.low_peak = self.low_peak.max(level);
                if self.low_millis >= LOW_MILLIS {
                    let lower = round_below(self.top, items);
                    if self.low_peak * HEADROOM <= lower {
                        self.top = lower;
                    }
                    self.reset_low();
                }
            } else {
                self.reset_low();
            }
        }
    }

    fn reset_low(&mut self) {
        self.low_millis = 0;
        self.low_peak = 0.0;
    }

    /// The chart's top: one unit before any activity.
    pub(super) fn top(&self) -> f64 {
        if self.top > 0.0 { self.top } else { 1.0 }
    }

    /// "200 MiB/s", "50 items/s": the top in the unit of its round value.
    pub(super) fn label(&self, items: bool, locale: &NumberLocale) -> String {
        let top = self.top();
        if items {
            return format!("{} items/s", whole(top, locale));
        }
        let (unit, size) = BYTE_UNITS
            .iter()
            .rev()
            .find(|(_, size)| top >= *size)
            .copied()
            .unwrap_or(BYTE_UNITS[0]);
        format!("{} {unit}/s", whole(top / size, locale))
    }
}

fn whole(value: f64, locale: &NumberLocale) -> String {
    locale.format_u64(value.round().clamp(0.0, u64::MAX as f64) as u64)
}

/// The round values in increasing order.
fn ladder(items: bool) -> impl Iterator<Item = f64> {
    let units: Vec<f64> = if items {
        (0..19).map(|power| 1000_f64.powi(power)).collect()
    } else {
        // TiB/s and beyond keep counting in TiB.
        BYTE_UNITS
            .iter()
            .map(|(_, size)| *size)
            .chain((1..6).map(|power| BYTE_UNITS[4].1 * 1000_f64.powi(power)))
            .collect()
    };
    units
        .into_iter()
        .flat_map(|unit| MULTIPLES.map(|multiple| multiple * unit))
}

/// The smallest round value at least `value`; beyond the ladder, `value` itself.
fn round_up(value: f64, items: bool) -> f64 {
    ladder(items)
        .find(|step| *step >= value)
        .unwrap_or(value)
        .max(1.0)
}

/// The largest round value below `value`, never below one unit.
fn round_below(value: f64, items: bool) -> f64 {
    ladder(items)
        .take_while(|step| *step < value)
        .last()
        .unwrap_or(1.0)
}
