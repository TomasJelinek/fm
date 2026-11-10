use super::*;

fn counts(transfer: u64, verification: u64, items: u64) -> Counters {
    Counters {
        transfer,
        verification,
        items,
    }
}

#[test]
fn measures_distinct_counters_and_stalled_intervals() {
    let mut sampler = Sampler::default();
    sampler.observe(Duration::ZERO, counts(0, 0, 0), true);
    let snapshot = sampler.observe(Duration::from_millis(250), counts(1000, 500, 2), true);
    assert_eq!(snapshot.transfer_per_second, 4000.0);
    assert_eq!(snapshot.verification_per_second, 2000.0);
    assert_eq!(snapshot.items_per_second, 8.0);
    let stalled = sampler.observe(Duration::from_millis(500), counts(1000, 500, 2), true);
    assert_eq!(stalled.transfer_per_second, 0.0);
    assert_eq!(stalled.samples.len(), 2);
}

#[test]
fn short_completion_retains_activity_but_stops_current_rates() {
    let mut sampler = Sampler::default();
    sampler.observe(Duration::ZERO, counts(0, 0, 0), true);
    let done = sampler.observe(Duration::from_millis(10), counts(1000, 500, 2), false);
    assert_eq!(done.transfer_per_second, 0.0);
    assert_eq!(done.verification_per_second, 0.0);
    assert_eq!(done.items_per_second, 0.0);
    assert_eq!(done.samples.len(), 1);
    assert_eq!(done.samples[0].transfer_per_second, 100_000.0);
    assert_eq!(done.samples[0].verification_per_second, 50_000.0);
    assert_eq!(done.samples[0].items_per_second, 200.0);
}

#[test]
fn waits_resume_without_counting_wait_time_and_counter_reset_does_not_spike() {
    let mut sampler = Sampler::default();
    sampler.observe(Duration::ZERO, counts(0, 0, 0), true);
    sampler.observe(Duration::from_millis(250), counts(1000, 0, 0), true);
    let paused = sampler.observe(Duration::from_millis(300), counts(1100, 0, 0), false);
    assert_eq!(paused.transfer_per_second, 0.0);
    sampler.observe(Duration::from_secs(10), counts(1100, 0, 0), false);
    sampler.observe(Duration::from_secs(10), counts(0, 0, 0), true);
    let resumed = sampler.observe(Duration::from_millis(10250), counts(1000, 0, 0), true);
    assert_eq!(resumed.transfer_per_second, 4000.0);
    let reset = sampler.observe(Duration::from_millis(10500), counts(1, 0, 0), true);
    assert_eq!(reset.transfer_per_second, 0.0);
    let retried = sampler.observe(Duration::from_millis(10750), counts(501, 0, 0), true);
    assert_eq!(retried.transfer_per_second, 2000.0);
}

#[test]
fn sampling_is_paced_bounded_and_keeps_the_entire_operation() {
    let mut sampler = Sampler::default();
    let initial = sampler.observe(Duration::ZERO, counts(0, 0, 0), false);
    let waiting = sampler.observe(Duration::from_secs(1), counts(0, 0, 0), false);
    assert!(Arc::ptr_eq(&initial, &waiting));
    sampler.observe(Duration::from_secs(1), counts(0, 0, 0), true);
    let first = sampler.observe(Duration::from_millis(1250), counts(1000, 0, 0), true);
    let early = sampler.observe(Duration::from_millis(1300), counts(1100, 0, 0), true);
    assert!(Arc::ptr_eq(&first, &early));
    for tick in 2..=400 {
        let snapshot = sampler.observe(
            Duration::from_millis(1000 + tick * 250),
            counts(tick * 1000, 0, 0),
            true,
        );
        assert!(snapshot.samples.len() <= 240);
        assert!(
            snapshot
                .samples
                .windows(2)
                .all(|pair| pair[1].elapsed_millis - pair[0].elapsed_millis >= 250)
        );
    }
    let stalled = sampler.observe(Duration::from_secs(200), counts(400_000, 0, 0), true);
    assert!(stalled.samples.len() > 1 && stalled.samples.len() <= 240);
    assert_eq!(stalled.samples[0].start_millis, 1000);
    assert_eq!(stalled.samples.last().unwrap().elapsed_millis, 200_000);
    assert_eq!(stalled.transfer_per_second, 0.0);
}

#[test]
fn final_partial_interval_merges_without_an_extra_high_frequency_sample() {
    let mut sampler = Sampler::default();
    sampler.observe(Duration::ZERO, counts(0, 0, 0), true);
    sampler.observe(Duration::from_millis(250), counts(1000, 0, 0), true);
    let done = sampler.observe(Duration::from_millis(300), counts(1200, 0, 0), false);
    assert_eq!(done.samples.len(), 1);
    assert_eq!(done.samples[0].transfer_per_second, 4000.0);
    assert_eq!(done.samples[0].elapsed_millis, 300);
}

#[test]
fn simultaneous_observations_and_regressing_time_are_finite() {
    let mut sampler = Sampler::default();
    sampler.observe(Duration::from_millis(1), counts(0, 0, 0), true);
    let zero = sampler.observe(Duration::from_millis(1), counts(1000, 0, 0), false);
    assert_eq!(zero.transfer_per_second, 0.0);
    assert!(
        zero.samples
            .iter()
            .all(|sample| sample.transfer_per_second.is_finite())
    );
    let earlier = sampler.observe(Duration::ZERO, counts(u64::MAX, 0, 0), false);
    assert_eq!(earlier.transfer_per_second, 0.0);
}

#[test]
fn move_proof_reads_only_increase_verification_activity() {
    let mut progress = CopyProgress::queued(
        midnight_core::OperationId(1),
        Default::default(),
        1,
        midnight_core::TransferIntent::Move,
    );
    let mut sampler = Sampler::default();
    sampler.observe(Duration::ZERO, copy_counters(&progress), true);
    progress.copied_bytes = 1000;
    progress.verified_bytes = 500;
    progress.validation_bytes = 750;
    let snapshot = sampler.observe(Duration::from_millis(250), copy_counters(&progress), true);
    assert_eq!(snapshot.transfer_per_second, 4000.0);
    assert_eq!(snapshot.verification_per_second, 5000.0);
    assert_eq!(snapshot.items_per_second, 0.0);
}

#[test]
fn cancellation_retains_counters_returned_by_a_previously_blocked_native_call() {
    let mut sampler = Sampler::default();
    sampler.observe(Duration::ZERO, counts(0, 0, 0), true);
    sampler.observe(Duration::from_millis(250), counts(0, 0, 0), false);
    let done = sampler.observe(Duration::from_millis(275), counts(1000, 500, 2), false);
    assert_eq!(done.transfer_per_second, 0.0);
    assert_eq!(done.items_per_second, 0.0);
    assert!(
        done.samples
            .iter()
            .any(|sample| sample.transfer_per_second > 0.0)
    );
    assert!(
        done.samples
            .iter()
            .any(|sample| sample.items_per_second > 0.0)
    );
}

#[test]
fn three_hours_keep_the_first_activity_and_each_series_peak_in_bounded_uniform_buckets() {
    let mut sampler = Sampler::default();
    let mut counters = counts(0, 0, 0);
    sampler.observe(Duration::ZERO, counters, true);
    let mut snapshot = Arc::default();
    for tick in 1..=43_200 {
        counters.transfer += if tick == 1 { 32_000 } else { 250 };
        counters.verification += if tick == 2 { 64_000 } else { 125 };
        counters.items += if tick == 3 { 20 } else { 1 };
        snapshot = sampler.observe(Duration::from_millis(tick * 250), counters, true);
        assert!(snapshot.samples.len() <= 240);
    }
    assert_eq!(snapshot.samples[0].start_millis, 0);
    assert!(
        snapshot.samples[0].elapsed_millis <= 120_000,
        "early activity must remain localized near the beginning, not collapsed across most of the job"
    );
    assert_eq!(snapshot.samples.last().unwrap().elapsed_millis, 10_800_000);
    assert!(
        snapshot
            .samples
            .windows(2)
            .all(|pair| pair[0].elapsed_millis == pair[1].start_millis)
    );
    assert!(
        snapshot
            .samples
            .iter()
            .all(|sample| sample.start_millis < sample.elapsed_millis)
    );
    assert_eq!(
        snapshot
            .samples
            .iter()
            .map(|sample| sample.transfer_per_second)
            .fold(0.0, f64::max),
        128_000.0
    );
    assert_eq!(
        snapshot
            .samples
            .iter()
            .map(|sample| sample.verification_per_second)
            .fold(0.0, f64::max),
        256_000.0
    );
    assert_eq!(
        snapshot
            .samples
            .iter()
            .map(|sample| sample.items_per_second)
            .fold(0.0, f64::max),
        80.0
    );
    assert_eq!(snapshot.transfer_per_second, 1000.0);
    assert_eq!(snapshot.verification_per_second, 500.0);
    assert_eq!(snapshot.items_per_second, 4.0);
    let done = sampler.observe(Duration::from_secs(10_800), counters, false);
    assert_eq!(done.transfer_per_second, 0.0);
    assert_eq!(done.samples[0].transfer_per_second, 128_000.0);
    assert!(Arc::ptr_eq(&snapshot.samples, &done.samples));
}

#[test]
fn partially_filled_compacted_bucket_includes_new_activity_but_current_speed_can_stop() {
    let mut sampler = Sampler::default();
    sampler.observe(Duration::ZERO, counts(0, 0, 0), true);
    for tick in 1..=240 {
        sampler.observe(Duration::from_millis(tick * 250), counts(0, 0, 0), true);
    }
    let newest = sampler.observe(Duration::from_millis(60_250), counts(1000, 0, 0), true);
    assert_eq!(newest.samples.last().unwrap().start_millis, 60_000);
    assert_eq!(newest.samples.last().unwrap().elapsed_millis, 60_250);
    assert_eq!(newest.samples.last().unwrap().transfer_per_second, 4000.0);
    let idle = sampler.observe(Duration::from_millis(60_500), counts(1000, 0, 0), true);
    assert_eq!(idle.transfer_per_second, 0.0);
    assert_eq!(idle.samples.last().unwrap().transfer_per_second, 4000.0);
    assert_eq!(idle.samples.last().unwrap().start_millis, 60_000);
    assert_eq!(idle.samples.last().unwrap().elapsed_millis, 60_500);
    let done = sampler.observe(Duration::from_millis(60_550), counts(1001, 0, 0), false);
    assert_eq!(done.transfer_per_second, 0.0);
    assert_eq!(done.samples.last().unwrap().transfer_per_second, 4000.0);
    assert_eq!(done.samples.last().unwrap().elapsed_millis, 60_550);
    assert_eq!(done.samples[0].start_millis, 0);
}

#[test]
fn final_partial_interval_never_lowers_an_already_observed_peak() {
    let mut sampler = Sampler::default();
    sampler.observe(Duration::ZERO, counts(0, 0, 0), true);
    sampler.observe(Duration::from_millis(250), counts(1000, 0, 0), true);
    let done = sampler.observe(Duration::from_millis(300), counts(1100, 0, 0), false);
    assert_eq!(done.samples[0].start_millis, 0);
    assert_eq!(done.samples[0].elapsed_millis, 300);
    assert_eq!(done.samples[0].transfer_per_second, 4000.0);
    assert_eq!(done.transfer_per_second, 0.0);
}

#[test]
fn compaction_preserves_actual_irregular_interval_boundaries() {
    let mut sampler = Sampler::default();
    sampler.observe(Duration::ZERO, counts(0, 0, 0), true);
    sampler.observe(Duration::from_millis(250), counts(1000, 0, 0), true);
    sampler.observe(Duration::from_millis(1250), counts(2000, 0, 0), true);
    for tick in 3..=240 {
        sampler.observe(
            Duration::from_millis(1250 + (tick - 2) * 250),
            counts(2000 + (tick - 2) * 250, 0, 0),
            true,
        );
    }
    let latest = sampler.observe(Duration::from_millis(61_000), counts(61_750, 0, 0), true);
    assert_eq!(latest.samples[0].start_millis, 0);
    assert_eq!(latest.samples[0].elapsed_millis, 1250);
    assert_eq!(latest.samples[0].transfer_per_second, 4000.0);
    assert_eq!(latest.samples[1].start_millis, 1250);
    assert_eq!(latest.samples.last().unwrap().start_millis, 60_750);
    assert_eq!(latest.samples.last().unwrap().elapsed_millis, 61_000);
    assert_eq!(latest.transfer_per_second, 1000.0);
}
