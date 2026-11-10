use super::*;
use midnight_core::ThroughputSample;

fn snapshot() -> Arc<OperationThroughput> {
    Arc::new(OperationThroughput {
        samples: vec![
            ThroughputSample {
                start_millis: 0,
                elapsed_millis: 250,
                transfer_per_second: 1024.0,
                verification_per_second: 0.0,
                items_per_second: 2.0,
            },
            ThroughputSample {
                start_millis: 250,
                elapsed_millis: 1000,
                transfer_per_second: 0.0,
                verification_per_second: 2048.0,
                items_per_second: 0.5,
            },
        ]
        .into(),
        transfer_per_second: 0.0,
        verification_per_second: 2048.0,
        items_per_second: 0.5,
    })
}

#[test]
fn native_move_uses_item_rate_without_claiming_written_or_verified_bytes() {
    let mut source = (*snapshot()).clone();
    source.samples = source
        .samples
        .iter()
        .map(|sample| ThroughputSample {
            transfer_per_second: 0.0,
            verification_per_second: 0.0,
            ..*sample
        })
        .collect::<Vec<_>>()
        .into();
    source.transfer_per_second = 0.0;
    source.verification_per_second = 0.0;
    let chart = build(&source, false, &NumberLocale::default());
    assert!(chart.native_move);
    assert!(chart.speed.ends_with("items/s"));
    assert!(chart.description.starts_with("Renamed:"));
    assert!(!chart.description.contains("B/s"));
}

#[test]
fn graph_uses_time_spacing_and_keeps_writes_separate_from_readback() {
    let chart = build(&snapshot(), false, &NumberLocale::default());
    assert!(chart.has_samples);
    assert_eq!(chart.speed, "0 B/s");
    assert_eq!(chart.verification_speed, "2.0 KiB/s");
    assert!(chart.primary.contains("L 250.00"));
    assert!(chart.primary.contains("L 1000.00 100.00"));
    assert!(chart.verification.contains("L 1000.00 9.09"));
    assert!(chart.area.ends_with("L 1000.00 100 Z"));
    assert_eq!(chart.history, "Elapsed 1 s");
}

#[test]
fn deletion_is_fractional_items_and_respects_locale() {
    let locale = NumberLocale {
        decimal_separator: ",".into(),
        ..Default::default()
    };
    let chart = build(&snapshot(), true, &locale);
    assert_eq!(chart.speed, "0,5 items/s");
    assert!(chart.description.starts_with("Deleted: 0,5 items/s"));
    assert!(!chart.description.contains("B/s"));
    assert!(chart.primary.contains("L 250.00 9.09"));
}

#[test]
fn terminal_zero_rates_keep_graph_and_cache_drops_obsolete_snapshots() {
    let cache = Charts::default();
    let before = snapshot();
    let weak = Arc::downgrade(&before);
    let a = cache.get(OperationId(1), false, &before, &NumberLocale::default());
    let b = cache.get(OperationId(1), false, &before, &NumberLocale::default());
    assert_eq!(a, b);
    assert_eq!(Arc::strong_count(&before), 2);
    let terminal = Arc::new(OperationThroughput {
        samples: before.samples.clone(),
        ..Default::default()
    });
    drop(before);
    let c = cache.get(OperationId(1), false, &terminal, &NumberLocale::default());
    assert!(weak.upgrade().is_none());
    assert_eq!(c.area, a.area);
    assert_eq!(c.verification_speed, "0 B/s");
    let separate = cache.get(OperationId(2), true, &snapshot(), &NumberLocale::default());
    assert_eq!(separate.speed, "0.5 items/s");
    assert_eq!(c.speed, "0 B/s");
}

#[test]
fn geometry_stays_bounded_and_empty_or_nonfinite_values_are_safe() {
    let empty = build(
        &OperationThroughput::default(),
        false,
        &NumberLocale::default(),
    );
    assert!(!empty.has_samples);
    assert!(empty.area.is_empty());
    let source = OperationThroughput {
        samples: (0..1000)
            .map(|index| ThroughputSample {
                elapsed_millis: index * 250,
                transfer_per_second: if index % 2 == 0 { f64::NAN } else { 123.0 },
                verification_per_second: f64::INFINITY,
                ..Default::default()
            })
            .collect::<Vec<_>>()
            .into(),
        transfer_per_second: f64::NEG_INFINITY,
        ..Default::default()
    };
    let chart = build(&source, false, &NumberLocale::default());
    assert_eq!(chart.primary.matches(" L ").count(), 240);
    assert!(!chart.area.contains("NaN"));
    assert!(!chart.area.contains("inf"));
    assert_eq!(chart.history, "Elapsed 4 min 09 s");
    assert_eq!(chart.speed, "0 B/s");
}

#[test]
fn removing_a_job_releases_its_chart_history() {
    let cache = Charts::default();
    let source = snapshot();
    let weak = Arc::downgrade(&source);
    cache.get(OperationId(9), false, &source, &NumberLocale::default());
    drop(source);
    cache.retain(std::iter::empty());
    assert!(weak.upgrade().is_none());
}

#[test]
fn submillisecond_completed_work_has_a_visible_point_without_inventing_duration() {
    let source = OperationThroughput {
        samples: vec![ThroughputSample {
            elapsed_millis: 0,
            transfer_per_second: 1024.0,
            ..Default::default()
        }]
        .into(),
        ..Default::default()
    };
    let chart = build(&source, false, &NumberLocale::default());
    assert!(chart.single_sample && chart.has_samples);
    assert_eq!(chart.point_x, 0.0);
    assert!(chart.point_y < 100.0);
    assert_eq!(chart.speed, "0 B/s");
}

#[test]
fn entire_history_shrinks_horizontally_and_retains_old_peaks_in_vertical_scale() {
    let mut source = OperationThroughput {
        samples: vec![
            ThroughputSample {
                start_millis: 0,
                elapsed_millis: 3_600_000,
                transfer_per_second: 8192.0,
                ..Default::default()
            },
            ThroughputSample {
                start_millis: 3_600_000,
                elapsed_millis: 7_200_000,
                transfer_per_second: 1024.0,
                ..Default::default()
            },
        ]
        .into(),
        transfer_per_second: 1024.0,
        ..Default::default()
    };
    let before = build(&source, false, &NumberLocale::default());
    assert!(before.primary.starts_with("M 0.00 9.09 L 500.00 9.09"));
    assert_eq!(before.history, "Elapsed 2 h 00 min 00 s");
    assert_eq!(before.speed, "1.0 KiB/s");
    let mut samples = source.samples.to_vec();
    samples.push(ThroughputSample {
        start_millis: 7_200_000,
        elapsed_millis: 14_400_000,
        transfer_per_second: 16384.0,
        ..Default::default()
    });
    source.samples = samples.into();
    let after = build(&source, false, &NumberLocale::default());
    assert!(after.primary.starts_with("M 0.00 54.55 L 250.00 54.55"));
    assert!(after.primary.ends_with("L 1000.00 9.09"));
    assert_eq!(after.history, "Elapsed 4 h 00 min 00 s");
    assert_eq!(
        after.speed, "1.0 KiB/s",
        "historical peaks are not current speed"
    );
}
