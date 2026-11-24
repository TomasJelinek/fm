use super::*;
use midnight_core::ThroughputSample;
use scale::Scale;

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
        ..Default::default()
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
    let chart = build(
        &source,
        View::Transfer,
        &NumberLocale::default(),
        &mut Scale::default(),
    );
    assert!(chart.native_move);
    assert!(chart.speed.ends_with("items/s"));
    assert!(chart.description.starts_with("Renamed:"));
    assert!(!chart.description.contains("B/s"));
}

#[test]
fn graph_uses_time_spacing_and_keeps_writes_separate_from_readback() {
    let chart = build(
        &snapshot(),
        View::Transfer,
        &NumberLocale::default(),
        &mut Scale::default(),
    );
    assert!(chart.has_samples);
    assert_eq!(chart.speed, "0 B/s");
    assert_eq!(chart.verification_speed, "2.0 KiB/s");
    assert!(chart.primary.contains("L 250.00"));
    assert!(chart.primary.contains("L 1000.00 100.00"));
    assert!(chart.verification.contains("L 1000.00 0.00"));
    assert!(chart.area.ends_with("L 1000.00 100 Z"));
    assert_eq!(chart.history, "Elapsed 1 s");
}

#[test]
fn deletion_is_fractional_items_and_respects_locale() {
    let locale = NumberLocale {
        decimal_separator: ",".into(),
        ..Default::default()
    };
    let chart = build(&snapshot(), View::Deletion, &locale, &mut Scale::default());
    assert_eq!(chart.speed, "0,5 items/s");
    assert!(chart.description.starts_with("Deleted: 0,5 items/s"));
    assert!(!chart.description.contains("B/s"));
    assert!(chart.primary.contains("L 250.00 60.00"));
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
        &mut Scale::default(),
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
    let chart = build(
        &source,
        View::Transfer,
        &NumberLocale::default(),
        &mut Scale::default(),
    );
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
    let chart = build(
        &source,
        View::Transfer,
        &NumberLocale::default(),
        &mut Scale::default(),
    );
    assert!(chart.single_sample && chart.has_samples);
    assert_eq!(chart.point_x, 0.0);
    assert!(chart.point_y < 100.0);
    assert_eq!(chart.speed, "0 B/s");
}

#[test]
fn entire_history_shrinks_horizontally_and_its_peaks_set_the_scale() {
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
    let before = build(
        &source,
        View::Transfer,
        &NumberLocale::default(),
        &mut Scale::default(),
    );
    // 8 KiB/s raised the scale to 10 KiB/s; an hour at 1 KiB/s lowered it one step.
    assert_eq!(before.scale, "5 KiB/s");
    assert!(before.primary.starts_with("M 0.00 0.00 L 500.00 0.00"));
    assert!(before.primary.ends_with("L 1000.00 80.00"));
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
    let after = build(
        &source,
        View::Transfer,
        &NumberLocale::default(),
        &mut Scale::default(),
    );
    assert_eq!(after.scale, "20 KiB/s");
    assert!(after.primary.starts_with("M 0.00 60.00 L 250.00 60.00"));
    assert!(after.primary.ends_with("L 1000.00 20.00"));
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
        ..Default::default()
    });
    let cache = Charts::default();
    let locale = NumberLocale::default();
    cache.get(OperationId(1), false, &source, &locale);
    let chart = cache.get_verification(OperationId(1), 1, &source, &locale);

    assert!(!chart.native_move);
    assert_eq!(chart.speed, "512 B/s");
    assert_eq!(chart.scale, "2 KiB/s");
    assert_eq!(chart.primary, "M 0.00 50.00 L 1000.00 50.00");
    assert_eq!(
        chart.area,
        "M 0.00 100 L 0.00 50.00 L 1000.00 50.00 L 1000.00 100 Z"
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
    assert_eq!(localized.scale, "5 KiB/s");

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

const MIB: f64 = 1024.0 * 1024.0;

/// `count` consecutive samples of `step` milliseconds each, at `rate(index)` bytes
/// per second (index from 1); a long job's history arrives combined like this.
fn writes(count: u64, step: u64, rate: impl Fn(u64) -> f64) -> OperationThroughput {
    OperationThroughput {
        samples: (1..=count)
            .map(|index| ThroughputSample {
                start_millis: (index - 1) * step,
                elapsed_millis: index * step,
                transfer_per_second: rate(index),
                ..Default::default()
            })
            .collect::<Vec<_>>()
            .into(),
        transfer_per_second: rate(count),
        ..Default::default()
    }
}

/// The chart's top after each report of a job that samples every 250 ms (as the
/// backend does) and reports every `report` milliseconds, for `seconds` of
/// `rate(millis)` bytes per second; the presenter's cache sees each report once.
fn tops(seconds: u64, report: u64, rate: impl Fn(u64) -> f64) -> Vec<(u64, f64)> {
    let cache = Charts::default();
    let locale = NumberLocale::default();
    let mut samples: Vec<ThroughputSample> = Vec::new();
    let mut tops: Vec<(u64, f64)> = Vec::new();
    let mut next_sample = 250;
    let mut now = 0;
    while now < seconds * 1000 {
        now += report;
        while next_sample <= now {
            samples.push(ThroughputSample {
                start_millis: next_sample - 250,
                elapsed_millis: next_sample,
                transfer_per_second: rate(next_sample),
                ..Default::default()
            });
            next_sample += 250;
        }
        if samples.is_empty() {
            continue;
        }
        // The backend keeps the newest 240 intervals.
        let kept = samples[samples.len().saturating_sub(240)..].to_vec();
        let source = Arc::new(OperationThroughput {
            samples: kept.into(),
            ..Default::default()
        });
        cache.get(OperationId(1), false, &source, &locale);
        let top = cache.top(OperationId(1), false).expect("a cached chart");
        if tops.last().is_none_or(|(_, last)| *last != top) {
            tops.push((now, top));
        }
    }
    tops
}

/// Rescales after the first scale, and the scales in order.
fn rescales(tops: &[(u64, f64)]) -> usize {
    tops.len().saturating_sub(1)
}

#[test]
fn a_steady_noisy_copy_keeps_one_scale() {
    // 120 MiB/s ±20%, changing every quarter second, for two minutes.
    let noisy =
        |millis: u64| 120.0 * MIB * (1.0 + 0.2 * ((millis / 250 * 7919 % 11) as f64 / 5.0 - 1.0));
    for report in [125, 250, 1000] {
        let tops = tops(120, report, noisy);
        assert_eq!(tops.len(), 1, "report every {report} ms: {tops:?}");
        assert_eq!(tops[0].1, 200.0 * MIB);
    }
}

#[test]
fn spikes_raise_the_scale_at_once_and_it_then_holds_without_oscillating() {
    // 100 MiB/s with a quarter second at 600 MiB/s every three seconds.
    let spiky = |millis: u64| {
        if millis.is_multiple_of(3000) {
            600.0 * MIB
        } else {
            100.0 * MIB
        }
    };
    for report in [125, 250, 1000] {
        let tops = tops(120, report, spiky);
        let scales: Vec<f64> = tops.iter().map(|(_, top)| *top).collect();
        assert_eq!(
            scales,
            [200.0 * MIB, 1024.0 * MIB],
            "report every {report} ms"
        );
        // Up at the first spike, within one report.
        assert!(tops[1].0 <= 3000 + report, "{tops:?}");
    }
}

#[test]
fn a_single_spike_steps_down_one_round_value_at_a_time_ten_seconds_apart() {
    // 20 MiB/s, with a quarter second at 1 GiB/s ten seconds in.
    let spike = |millis: u64| {
        if millis == 10_000 {
            1024.0 * MIB
        } else {
            20.0 * MIB
        }
    };
    for report in [125, 250, 1000] {
        let tops = tops(120, report, spike);
        let scales: Vec<f64> = tops.iter().map(|(_, top)| *top).collect();
        assert_eq!(
            scales,
            [
                50.0 * MIB,
                2048.0 * MIB,
                1024.0 * MIB,
                500.0 * MIB,
                200.0 * MIB,
                100.0 * MIB,
                50.0 * MIB,
            ],
            "report every {report} ms"
        );
        for pair in tops[1..].windows(2) {
            assert!(pair[1].0 - pair[0].0 >= 10_000, "{tops:?}");
        }
        assert!(rescales(&tops) <= 6);
    }
}

#[test]
fn a_ramp_down_lowers_the_scale_rarely_and_never_raises_it() {
    // From 400 MiB/s down to 10 MiB/s over two minutes, then steady.
    let ramp = |millis: u64| {
        let share = (millis as f64 / 120_000.0).min(1.0);
        (400.0 - 390.0 * share) * MIB
    };
    let ramped = tops(180, 125, ramp);
    assert_eq!(ramped[0].1, 500.0 * MIB);
    assert!(rescales(&ramped) <= 6, "{ramped:?}");
    for pair in ramped.windows(2) {
        assert!(pair[1].1 < pair[0].1, "never up: {ramped:?}");
        assert!(
            pair[1].0 - pair[0].0 >= 10_000,
            "ten seconds apart: {ramped:?}"
        );
    }
    assert_eq!(ramped.last().unwrap().1, 20.0 * MIB, "{ramped:?}");
    // Each report interval gives the same scales.
    assert_eq!(
        tops(180, 1000, ramp)
            .iter()
            .map(|(_, top)| *top)
            .collect::<Vec<_>>(),
        ramped.iter().map(|(_, top)| *top).collect::<Vec<_>>()
    );
}

#[test]
fn alternating_speeds_and_pauses_hold_the_scale() {
    // Five seconds at 100 MiB/s, five at 50 MiB/s, and a minute idle in between.
    let alternating = |millis: u64| match millis {
        60_001..=120_000 => 0.0,
        _ if (millis / 5000).is_multiple_of(2) => 100.0 * MIB,
        _ => 50.0 * MIB,
    };
    let tops = tops(180, 250, alternating);
    assert_eq!(tops.len(), 1, "{tops:?}");
    assert_eq!(tops[0].1, 200.0 * MIB);
}

#[test]
fn scale_labels_are_round_values_of_their_unit() {
    let locale = NumberLocale::default();
    let chart = |rate: f64, deletion: bool| {
        let source = OperationThroughput {
            samples: vec![ThroughputSample {
                elapsed_millis: 250,
                transfer_per_second: rate,
                items_per_second: rate,
                ..Default::default()
            }]
            .into(),
            ..Default::default()
        };
        let view = if deletion {
            View::Deletion
        } else {
            View::Transfer
        };
        build(&source, view, &locale, &mut Scale::default()).scale
    };
    assert_eq!(chart(150.0 * MIB, false), "200 MiB/s");
    assert_eq!(chart(500.0 * MIB, false), "1 GiB/s");
    assert_eq!(chart(300.0, false), "500 B/s");
    assert_eq!(chart(900.0, false), "2 KiB/s");
    assert_eq!(chart(1.0, false), "2 B/s");
    assert_eq!(chart(30.0, true), "50 items/s");
    assert_eq!(chart(0.3, true), "1 items/s");
    assert_eq!(chart(3000.0, true), "5000 items/s");
}

#[test]
fn readback_shares_the_scale_and_a_clipped_peak_runs_along_the_top() {
    let locale = NumberLocale::default();
    let mut readback = writes(240, 250, |_| 30.0 * MIB);
    readback.samples = readback
        .samples
        .iter()
        .map(|sample| ThroughputSample {
            verification_per_second: 60.0 * MIB,
            ..*sample
        })
        .collect::<Vec<_>>()
        .into();
    let chart = build(&readback, View::Transfer, &locale, &mut Scale::default());
    assert_eq!(chart.scale, "100 MiB/s");
    assert!(chart.verification.ends_with("L 1000.00 40.00"));
    assert!(chart.primary.ends_with("L 1000.00 70.00"));
    // An older peak that the scale has come down from runs along the top edge.
    let spike = |tick| if tick == 40 { 1024.0 * MIB } else { 20.0 * MIB };
    let mut scale = Scale::default();
    let chart = build(
        &writes(240, 250, spike),
        View::Transfer,
        &locale,
        &mut scale,
    );
    assert_eq!(chart.scale, "50 MiB/s");
    assert!(chart.primary.contains("L 166.67 0.00"), "{}", chart.primary);
    assert!(chart.primary.ends_with("L 1000.00 60.00"));
}
