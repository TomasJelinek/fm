use super::*;
use midnight_core::{OperationId, TransferIntent};
use std::path::PathBuf;

fn progress() -> CopyProgress {
    let mut progress = CopyProgress::queued(
        OperationId(1),
        PathBuf::from("destination"),
        1,
        TransferIntent::Copy,
    );
    progress.state = CopyOperationState::Running;
    progress.phase = CopyOperationPhase::Verifying;
    progress.current_file_size = Some(1024);
    progress.verification_pass_id = 1;
    progress
}

fn observe(sampler: &mut PassSampler, progress: &mut CopyProgress, millis: u64) {
    sampler.observe(Duration::from_millis(millis), progress);
}

#[test]
fn fast_verification_flushes_on_publishing_boundary_before_a_timer_tick() {
    let mut sampler = PassSampler::default();
    let mut progress = progress();
    observe(&mut sampler, &mut progress, 100);
    progress.current_file_verified = 100;
    progress.phase = CopyOperationPhase::Publishing;
    observe(&mut sampler, &mut progress, 110);
    assert_eq!(progress.verification_throughput.samples.len(), 1);
    assert_eq!(
        progress.verification_throughput.samples[0].verification_per_second,
        10_000.0
    );
    assert_eq!(
        progress.verification_throughput.verification_per_second,
        0.0
    );
    assert_eq!(progress.verification_pass_id, 1);
}

#[test]
fn next_file_gets_an_empty_independent_history() {
    let mut sampler = PassSampler::default();
    let mut progress = progress();
    observe(&mut sampler, &mut progress, 0);
    progress.current_file_verified = 100;
    observe(&mut sampler, &mut progress, 250);
    let previous = progress.verification_throughput.clone();
    progress.verification_pass_id = 2;
    progress.current_file_verified = 0;
    observe(&mut sampler, &mut progress, 1000);
    assert!(progress.verification_throughput.samples.is_empty());
    assert_eq!(previous.samples.len(), 1);
    progress.current_file_verified = 20;
    observe(&mut sampler, &mut progress, 1250);
    assert_eq!(
        progress.verification_throughput.verification_per_second,
        80.0
    );
    assert_eq!(progress.verification_throughput.samples.len(), 1);
    assert_eq!(progress.verification_throughput.samples[0].start_millis, 0);
}

#[test]
fn retry_starts_at_retained_verified_offset_without_phantom_readback() {
    let mut sampler = PassSampler::default();
    let mut progress = progress();
    observe(&mut sampler, &mut progress, 0);
    progress.current_file_verified = 400;
    observe(&mut sampler, &mut progress, 250);
    progress.verification_pass_id = 2;
    observe(&mut sampler, &mut progress, 500);
    assert_eq!(progress.current_file_verified, 400);
    assert!(progress.verification_throughput.samples.is_empty());
    progress.current_file_verified = 450;
    observe(&mut sampler, &mut progress, 750);
    assert_eq!(
        progress.verification_throughput.verification_per_second,
        200.0
    );
}

#[test]
fn pause_resume_preserves_pass_and_history_but_excludes_idle_time() {
    let mut sampler = PassSampler::default();
    let mut progress = progress();
    observe(&mut sampler, &mut progress, 0);
    progress.current_file_verified = 100;
    progress.paused = true;
    observe(&mut sampler, &mut progress, 100);
    let history = progress.verification_throughput.samples.clone();
    assert_eq!(history.len(), 1);
    progress.paused = false;
    observe(&mut sampler, &mut progress, 10_000);
    progress.current_file_verified = 150;
    observe(&mut sampler, &mut progress, 10_250);
    assert_eq!(progress.verification_pass_id, 1);
    assert_eq!(progress.verification_throughput.samples.len(), 2);
    assert_eq!(
        progress.verification_throughput.samples[0].verification_per_second,
        history[0].verification_per_second
    );
    assert_eq!(
        progress.verification_throughput.verification_per_second,
        200.0
    );
}

#[test]
fn move_proof_reads_and_source_writes_do_not_enter_file_verification_history() {
    let mut sampler = PassSampler::default();
    let mut progress = progress();
    progress.intent = TransferIntent::Move;
    observe(&mut sampler, &mut progress, 0);
    progress.current_file_verified = 100;
    progress.phase = CopyOperationPhase::ValidatingAfterMove;
    observe(&mut sampler, &mut progress, 100);
    let before = progress.verification_throughput.clone();
    progress.validation_bytes = 1_000_000;
    progress.copied_bytes = 2_000_000;
    observe(&mut sampler, &mut progress, 1000);
    assert_eq!(
        progress.verification_throughput.verification_per_second,
        0.0
    );
    assert!(
        progress
            .verification_throughput
            .samples
            .iter()
            .all(|sample| sample.transfer_per_second == 0.0 && sample.items_per_second == 0.0)
    );
    assert_eq!(
        progress.verification_throughput.samples[0].verification_per_second,
        before.samples[0].verification_per_second
    );
}

#[test]
fn no_current_file_clears_id_and_history_including_empty_file_completion() {
    let mut sampler = PassSampler::default();
    let mut progress = progress();
    progress.current_file_size = Some(0);
    observe(&mut sampler, &mut progress, 0);
    assert_eq!(progress.verification_pass_id, 1);
    assert!(progress.verification_throughput.samples.is_empty());
    progress.current_file_size = None;
    observe(&mut sampler, &mut progress, 1);
    assert_eq!(progress.verification_pass_id, 0);
    assert!(progress.verification_throughput.samples.is_empty());
    assert_eq!(
        progress.verification_throughput.verification_per_second,
        0.0
    );
}

#[test]
fn pass_ids_are_nonzero_monotonic_and_exhaustion_never_reuses_an_id() {
    let mut ids = PassIds::default();
    assert_eq!(ids.next().unwrap(), 1);
    assert_eq!(ids.next().unwrap(), 2);
    ids.last = u64::MAX - 1;
    assert_eq!(ids.next().unwrap(), u64::MAX);
    assert!(ids.next().is_err());
    assert!(ids.next().is_err());
    assert_eq!(ids.last, u64::MAX);
}
