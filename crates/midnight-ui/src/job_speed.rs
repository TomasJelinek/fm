//! Presentation only: rates and timestamps are measured by the operation backend.
use crate::{SpeedChartData, format_size};
use midnight_core::{NumberLocale, OperationId, OperationThroughput};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    fmt::Write,
    sync::Arc,
};

#[derive(Default)]
pub(crate) struct Charts(RefCell<HashMap<(bool, OperationId), Cached>>);

struct Cached {
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
        let mut cache = self.0.borrow_mut();
        let item = cache.entry((deletion, id)).or_insert_with(|| Cached {
            source: source.clone(),
            locale: locale.clone(),
            chart: build(source, deletion, locale),
        });
        if !Arc::ptr_eq(&item.source, source) || item.locale != *locale {
            *item = Cached {
                source: source.clone(),
                locale: locale.clone(),
                chart: build(source, deletion, locale),
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

fn build(source: &OperationThroughput, deletion: bool, locale: &NumberLocale) -> SpeedChartData {
    let native_move = !deletion
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
        } else {
            sample.transfer_per_second
        })
    };
    let peak = samples.iter().fold(0.0_f64, |peak, sample| {
        peak.max(primary(sample)).max(if items {
            0.0
        } else {
            finite(sample.verification_per_second)
        })
    });
    let scale = if peak <= f64::MAX / 1.1 {
        (peak * 1.1).max(1.0)
    } else {
        peak
    };
    let mut line = String::new();
    let mut verification = String::new();
    let mut area = String::new();
    let mut last_x = 0.0;
    let mut last_y = 100.0;
    let mut last_verification_y = 100.0;
    for (index, sample) in samples.iter().enumerate() {
        let x = (sample.elapsed_millis as f64 / span * 1000.0).clamp(last_x, 1000.0);
        let y = 100.0 - primary(sample) / scale * 100.0;
        let verify_y = if deletion {
            100.0
        } else {
            100.0 - finite(sample.verification_per_second) / scale * 100.0
        };
        if index == 0 {
            let initial_x = (sample.start_millis as f64 / span * 1000.0).clamp(0.0, x);
            let _ = write!(line, "M {initial_x:.2} {y:.2}");
            let _ = write!(verification, "M {initial_x:.2} {verify_y:.2}");
            let _ = write!(area, "M {initial_x:.2} 100 L {initial_x:.2} {y:.2}");
        }
        let _ = write!(line, " L {x:.2} {y:.2}");
        let _ = write!(verification, " L {x:.2} {verify_y:.2}");
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
        } else {
            source.transfer_per_second
        },
        items,
        locale,
    );
    let verification_speed = rate(source.verification_per_second, false, locale);
    let history = format!("Elapsed {}", elapsed(end, locale));
    SpeedChartData {
        native_move,
        area: area.into(),
        primary: line.into(),
        verification: verification.into(),
        description: if deletion {
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
