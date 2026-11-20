//! Presentation only: rates and timestamps are measured by the operation backend.
use crate::{SpeedChartData, format_size};
use midnight_core::{NumberLocale, OperationId, OperationThroughput};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    fmt::Write,
    sync::Arc,
};

/// The last minute of samples sets the sustained level, or the last
/// `RECENT_SAMPLES` once long histories combine samples into longer intervals.
const RECENT_MILLIS: u64 = 60_000;
const RECENT_SAMPLES: usize = 12;
/// Nearest-rank percentile of those samples that the scale follows.
const SUSTAINED_PERCENTILE: f64 = 0.9;
/// A higher sample still raises the scale, with a weight that halves this often.
const PEAK_HALF_LIFE_MILLIS: f64 = 1000.0;
const HEADROOM: f64 = 1.1;

#[derive(Default)]
pub(crate) struct Charts(RefCell<HashMap<(bool, OperationId), Cached>>);

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Transfer,
    Deletion,
    Verification(u64),
}

struct Cached {
    view: View,
    source: Arc<OperationThroughput>,
    locale: NumberLocale,
    chart: SpeedChartData,
}

impl Charts {
    pub(crate) fn retain(&self, jobs: impl Iterator<Item = (bool, OperationId)>) {
        let retained: HashSet<_> = jobs.collect();
        self.0.borrow_mut().retain(|id, _| retained.contains(id));
    }

    pub(crate) fn get(
        &self,
        id: OperationId,
        deletion: bool,
        source: &Arc<OperationThroughput>,
        locale: &NumberLocale,
    ) -> SpeedChartData {
        self.get_view(
            id,
            if deletion {
                View::Deletion
            } else {
                View::Transfer
            },
            source,
            locale,
        )
    }

    /// `source` contains only this backend-identified readback attempt's history.
    /// Pause/resume retains its identity; each new file or retry supplies a fresh pass.
    /// Whole-operation samples cannot reconstruct this view after interval compaction.
    pub(crate) fn get_verification(
        &self,
        id: OperationId,
        pass_identity: u64,
        source: &Arc<OperationThroughput>,
        locale: &NumberLocale,
    ) -> SpeedChartData {
        self.get_view(id, View::Verification(pass_identity), source, locale)
    }

    fn get_view(
        &self,
        id: OperationId,
        view: View,
        source: &Arc<OperationThroughput>,
        locale: &NumberLocale,
    ) -> SpeedChartData {
        let mut cache = self.0.borrow_mut();
        // A new view replaces the copy job's slot rather than retaining every pass.
        // The backend continues to own the separate whole-operation history.
        let item = cache
            .entry((view == View::Deletion, id))
            .or_insert_with(|| Cached {
                view,
                source: source.clone(),
                locale: locale.clone(),
                chart: build(source, view, locale),
            });
        if item.view != view || !Arc::ptr_eq(&item.source, source) || item.locale != *locale {
            *item = Cached {
                view,
                source: source.clone(),
                locale: locale.clone(),
                chart: build(source, view, locale),
            };
        }
        item.chart.clone()
    }
}

fn finite(value: f64) -> f64 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn rate(value: f64, items: bool, locale: &NumberLocale) -> String {
    let value = finite(value).min(u64::MAX as f64);
    if items {
        let rounded = format!("{value:.1}");
        let (whole, fraction) = rounded.split_once('.').unwrap_or((&rounded, "0"));
        let whole = whole.parse().unwrap_or(u64::MAX);
        format!(
            "{}{}{fraction} items/s",
            locale.format_u64(whole),
            locale.decimal_separator
        )
    } else {
        format!("{}/s", format_size(value.round() as u64, locale))
    }
}

fn build(source: &OperationThroughput, view: View, locale: &NumberLocale) -> SpeedChartData {
    let deletion = view == View::Deletion;
    let verification_only = matches!(view, View::Verification(_));
    let native_move = view == View::Transfer
        && source
            .samples
            .iter()
            .any(|sample| sample.items_per_second > 0.0)
        && source.samples.iter().all(|sample| {
            sample.transfer_per_second == 0.0 && sample.verification_per_second == 0.0
        });
    let items = deletion || native_move;
    let end = source
        .samples
        .last()
        .map_or(0, |sample| sample.elapsed_millis);
    let span = end.max(1) as f64;
    let mut samples: Vec<_> = source
        .samples
        .iter()
        .rev()
        .take(240)
        .filter(|sample| sample.elapsed_millis <= end)
        .collect();
    samples.reverse();
    let primary = |sample: &midnight_core::ThroughputSample| {
        finite(if items {
            sample.items_per_second
        } else if verification_only {
            sample.verification_per_second
        } else {
            sample.transfer_per_second
        })
    };
    let levels: Vec<_> = samples
        .iter()
        .map(|sample| {
            let level = if items || verification_only {
                primary(sample)
            } else {
                primary(sample).max(finite(sample.verification_per_second))
            };
            (sample.elapsed_millis, level)
        })
        .collect();
    let scale = sustained_scale(&levels);
    // Above the scale a curve runs along the top edge: a clipped peak.
    let height = |rate: f64| (100.0 - rate / scale * 100.0).clamp(0.0, 100.0);
    let mut line = String::new();
    let mut verification = String::new();
    let mut area = String::new();
    let mut last_x = 0.0;
    let mut last_y = 100.0;
    let mut last_verification_y = 100.0;
    for (index, sample) in samples.iter().enumerate() {
        let x = (sample.elapsed_millis as f64 / span * 1000.0).clamp(last_x, 1000.0);
        let y = height(primary(sample));
        let verify_y = if deletion || verification_only {
            100.0
        } else {
            height(finite(sample.verification_per_second))
        };
        if index == 0 {
            let initial_x = (sample.start_millis as f64 / span * 1000.0).clamp(0.0, x);
            let _ = write!(line, "M {initial_x:.2} {y:.2}");
            if !verification_only {
                let _ = write!(verification, "M {initial_x:.2} {verify_y:.2}");
            }
            let _ = write!(area, "M {initial_x:.2} 100 L {initial_x:.2} {y:.2}");
        }
        let _ = write!(line, " L {x:.2} {y:.2}");
        if !verification_only {
            let _ = write!(verification, " L {x:.2} {verify_y:.2}");
        }
        let _ = write!(area, " L {x:.2} {y:.2}");
        last_x = x;
        last_y = y;
        last_verification_y = verify_y;
    }
    if !samples.is_empty() {
        let _ = write!(area, " L {last_x:.2} 100 Z");
    }
    let speed = rate(
        if items {
            source.items_per_second
        } else if verification_only {
            source.verification_per_second
        } else {
            source.transfer_per_second
        },
        items,
        locale,
    );
    let verification_speed = if verification_only {
        String::new()
    } else {
        rate(source.verification_per_second, false, locale)
    };
    let history = format!("Elapsed {}", elapsed(end, locale));
    SpeedChartData {
        native_move,
        area: area.into(),
        primary: line.into(),
        verification: verification.into(),
        description: if verification_only {
            format!("Verify: {speed}. {history}.")
        } else if deletion {
            format!("Deleted: {speed}. {history}.")
        } else if native_move {
            format!("Renamed: {speed}. No content copied. {history}.")
        } else {
            format!("Write: {speed}. Verify: {verification_speed}. {history}.")
        }
        .into(),
        speed: speed.into(),
        verification_speed: verification_speed.into(),
        scale: rate(scale, items, locale).into(),
        history: history.into(),
        has_samples: !samples.is_empty(),
        single_sample: samples.len() == 1,
        point_x: last_x as f32,
        point_y: last_y as f32,
        verification_y: last_verification_y as f32,
    }
}

/// The vertical scale follows sustained rates, not the history's peak: the 90th
/// percentile of the last minute's active samples, raised by any higher sample with
/// a weight that halves every second after it. A spike therefore lifts the scale
/// briefly, which then comes back down within seconds and draws the spike as a
/// clipped peak. Pauses, waits and stalls are recorded as idle (zero) intervals:
/// they neither pull the scale down nor age a peak, so it holds still through them.
/// `levels` are (end time, highest drawn rate) in time order.
fn sustained_scale(levels: &[(u64, f64)]) -> f64 {
    let active: Vec<_> = levels
        .iter()
        .copied()
        .filter(|(_, level)| *level > 0.0)
        .collect();
    let Some(&(now, _)) = active.last() else {
        return 1.0;
    };
    let older = active
        .iter()
        .rposition(|(at, _)| now.saturating_sub(*at) > RECENT_MILLIS)
        .map_or(0, |index| index + 1);
    let mut recent: Vec<f64> = active[older.min(active.len().saturating_sub(RECENT_SAMPLES))..]
        .iter()
        .map(|(_, level)| *level)
        .collect();
    let rank = (recent.len() as f64 * SUSTAINED_PERCENTILE).ceil() as usize;
    let index = rank.clamp(1, recent.len()) - 1;
    let sustained = *recent.select_nth_unstable_by(index, f64::total_cmp).1;
    let peak = active.iter().fold(0.0_f64, |peak, (at, level)| {
        let age = now.saturating_sub(*at) as f64;
        peak.max(level * 0.5_f64.powf(age / PEAK_HALF_LIFE_MILLIS))
    });
    let level = sustained.max(peak);
    if level <= f64::MAX / HEADROOM {
        (level * HEADROOM).max(1.0)
    } else {
        level
    }
}

fn elapsed(millis: u64, locale: &NumberLocale) -> String {
    if millis < 1000 {
        return format!("{} ms", locale.format_u64(millis));
    }
    let seconds = millis / 1000;
    if seconds < 60 {
        format!("{} s", locale.format_u64(seconds))
    } else if seconds < 3600 {
        format!(
            "{} min {:02} s",
            locale.format_u64(seconds / 60),
            seconds % 60
        )
    } else {
        format!(
            "{} h {:02} min {:02} s",
            locale.format_u64(seconds / 3600),
            seconds / 60 % 60,
            seconds % 60
        )
    }
}

#[cfg(test)]
#[path = "job_speed_tests.rs"]
mod tests;
