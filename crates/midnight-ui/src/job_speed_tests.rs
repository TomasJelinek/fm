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
    let chart = build(&source, View::Transfer, &NumberLocale::default());
    assert!(chart.native_move);
    assert!(chart.speed.ends_with("items/s"));
    assert!(chart.description.starts_with("Renamed:"));
    assert!(!chart.description.contains("B/s"));
}

#[test]
fn graph_uses_time_spacing_and_keeps_writes_separate_from_readback() {
    let chart = build(&snapshot(), View::Transfer, &NumberLocale::default());
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
    let chart = build(&snapshot(), View::Deletion, &locale);
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
        View::Transfer,
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
    let chart = build(&source, View::Transfer, &NumberLocale::default());
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
    let chart = build(&source, View::Transfer, &NumberLocale::default());
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
    let before = build(&source, View::Transfer, &NumberLocale::default());
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
    let after = build(&source, View::Transfer, &NumberLocale::default());
    assert!(after.primary.starts_with("M 0.00 54.55 L 250.00 54.55"));
    assert!(after.primary.ends_with("L 1000.00 9.09"));
    assert_eq!(after.history, "Elapsed 4 h 00 min 00 s");
    assert_eq!(
        after.speed, "1.0 KiB/s",
        "historical peaks are not current speed"
    );
}

#[test]
fn verification_view_uses_readback_for_its_only_series_and_scale() {
    let source = Arc::new(OperationThroughput {
        samples: vec![ThroughputSample {
            start_millis: 0,
            elapsed_millis: 250,
            transfer_per_second: 1_048_576.0,
            verification_per_second: 1024.0,
            items_per_second: 100.0,
        }]
        .into(),
        transfer_per_second: 1_048_576.0,
        verification_per_second: 512.0,
        items_per_second: 100.0,
    });
    let cache = Charts::default();
    let locale = NumberLocale::default();
    cache.get(OperationId(1), false, &source, &locale);
    let chart = cache.get_verification(OperationId(1), 1, &source, &locale);

    assert!(!chart.native_move);
    assert_eq!(chart.speed, "512 B/s");
    assert_eq!(chart.scale, "1.1 KiB/s");
    assert_eq!(chart.primary, "M 0.00 9.09 L 1000.00 9.09");
    assert_eq!(
        chart.area,
        "M 0.00 100 L 0.00 9.09 L 1000.00 9.09 L 1000.00 100 Z"
    );
    assert!(chart.verification.is_empty() && chart.verification_speed.is_empty());
    assert_eq!(chart.description, "Verify: 512 B/s. Elapsed 250 ms.");
}

#[test]
fn verification_view_does_not_infer_native_rename_from_item_activity() {
    let source = Arc::new(OperationThroughput {
        samples: vec![ThroughputSample {
            elapsed_millis: 250,
            items_per_second: 10.0,
            ..Default::default()
        }]
        .into(),
        items_per_second: 10.0,
        ..Default::default()
    });
    let cache = Charts::default();
    let locale = NumberLocale::default();
    assert!(
        cache
            .get(OperationId(1), false, &source, &locale)
            .native_move
    );
    let chart = cache.get_verification(OperationId(1), 1, &source, &locale);

    assert!(!chart.native_move);
    assert_eq!(chart.speed, "0 B/s");
    assert_eq!(chart.scale, "1 B/s");
    assert_eq!(chart.point_y, 100.0);
    assert!(chart.verification.is_empty());
}

#[test]
fn a_new_verification_pass_starts_empty_and_releases_the_previous_pass() {
    let cache = Charts::default();
    let source = snapshot();
    let weak = Arc::downgrade(&source);
    let locale = NumberLocale::default();
    assert!(
        cache
            .get_verification(OperationId(1), 1, &source, &locale)
            .has_samples
    );
    drop(source);

    let fresh = Arc::new(OperationThroughput::default());
    let chart = cache.get_verification(OperationId(1), 2, &fresh, &locale);
    assert!(
        weak.upgrade().is_none(),
        "old passes must not accumulate in the cache"
    );
    assert!(!chart.has_samples && !chart.single_sample);
    assert!(chart.primary.is_empty() && chart.area.is_empty() && chart.verification.is_empty());
    assert_eq!(chart.speed, "0 B/s");
    assert_eq!(chart.scale, "1 B/s");
    assert_eq!(chart.history, "Elapsed 0 ms");
}

#[test]
fn one_verification_pass_keeps_history_across_updates_pause_and_resume() {
    let cache = Charts::default();
    let locale = NumberLocale::default();
    let source = snapshot();
    let before = cache.get_verification(OperationId(1), 7, &source, &locale);
    assert_eq!(
        cache.get_verification(OperationId(1), 7, &source, &locale),
        before
    );
    assert_eq!(Arc::strong_count(&source), 2);

    let mut samples = source.samples.to_vec();
    samples.push(ThroughputSample {
        start_millis: 1000,
        elapsed_millis: 2000,
        verification_per_second: 4096.0,
        ..Default::default()
    });
    let running = Arc::new(OperationThroughput {
        samples: samples.into(),
        verification_per_second: 4096.0,
        ..Default::default()
    });
    let updated = cache.get_verification(OperationId(1), 7, &running, &locale);
    assert_eq!(updated.primary.matches(" L ").count(), 3);
    assert_eq!(updated.history, "Elapsed 2 s");
    assert_eq!(updated.speed, "4.0 KiB/s");

    let paused = Arc::new(OperationThroughput {
        samples: running.samples.clone(),
        ..Default::default()
    });
    let stopped = cache.get_verification(OperationId(1), 7, &paused, &locale);
    assert_eq!(stopped.primary, updated.primary);
    assert_eq!(stopped.area, updated.area);
    assert_eq!(stopped.speed, "0 B/s");
    assert_eq!(
        cache.get_verification(OperationId(1), 7, &paused, &locale),
        stopped
    );

    let resumed = cache.get_verification(OperationId(1), 7, &running, &locale);
    assert_eq!(resumed.primary, updated.primary);
    assert_eq!(resumed.speed, "4.0 KiB/s");
}

#[test]
fn whole_operation_history_is_restored_after_a_verification_view() {
    let cache = Charts::default();
    let source = snapshot();
    let locale = NumberLocale::default();
    let original = cache.get(OperationId(1), false, &source, &locale);
    let verification = cache.get_verification(OperationId(1), 1, &source, &locale);
    assert_eq!(verification.speed, "2.0 KiB/s");
    assert!(verification.verification.is_empty());

    let restored = cache.get(OperationId(1), false, &source, &locale);
    assert_eq!(restored, original);
    assert_eq!(restored.speed, "0 B/s");
    assert_eq!(restored.verification_speed, "2.0 KiB/s");
    assert!(!restored.verification.is_empty());
}

#[test]
fn verification_cache_refreshes_locale_and_releases_only_unretained_job_history() {
    let cache = Charts::default();
    let source = snapshot();
    let weak = Arc::downgrade(&source);
    let locale = NumberLocale::default();
    let initial = cache.get_verification(OperationId(9), 1, &source, &locale);
    assert_eq!(initial.speed, "2.0 KiB/s");
    let decimal_comma = NumberLocale {
        decimal_separator: ",".into(),
        ..Default::default()
    };
    let localized = cache.get_verification(OperationId(9), 1, &source, &decimal_comma);
    assert_eq!(localized.speed, "2,0 KiB/s");
    assert_eq!(localized.scale, "2,2 KiB/s");

    let deletion = snapshot();
    let deletion_weak = Arc::downgrade(&deletion);
    let separate = cache.get(OperationId(9), true, &deletion, &locale);
    assert_eq!(separate.speed, "0.5 items/s");
    drop(deletion);
    drop(source);
    cache.retain([(false, OperationId(9))].into_iter());
    assert!(deletion_weak.upgrade().is_none());
    assert!(weak.upgrade().is_some());
    cache.retain(std::iter::empty());
    assert!(weak.upgrade().is_none());
}
