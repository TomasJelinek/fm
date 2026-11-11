use super::*;
use midnight_core::{
    OperationId, OperationThroughput, TransferIntent, TransferRemainingWork, TransferVerification,
};
use std::sync::Arc;

fn transfer(intent: TransferIntent, verification: TransferVerification) -> CopyProgress {
    let mut p = CopyProgress::queued(OperationId(1), "target".into(), 1, intent);
    p.state = CopyOperationState::Running;
    p.verification = verification;
    p.discovery = OperationDiscovery {
        state: DiscoveryState::Complete,
        files: 1,
        bytes: Some(1_000_000),
        ..Default::default()
    };
    p.throughput = Arc::new(OperationThroughput {
        verification_per_second: 65_536.0,
        ..Default::default()
    });
    p
}
#[test]
fn sampled_copy_estimates_actual_readback_and_not_the_entire_file_again() {
    let mut p = transfer(TransferIntent::Copy, TransferVerification::SizeAndSamples);
    p.current_file_copied = 1_000_000;
    p.remaining_work = Some(TransferRemainingWork {
        write_bytes: 0,
        verification_bytes: 196_608,
    });
    assert_eq!(copy(&p, &NumberLocale::default()).eta, "About 3 s");
    p.current_file_verified = 65_536;
    p.remaining_work.as_mut().unwrap().verification_bytes = 131_072;
    assert_eq!(copy(&p, &NumberLocale::default()).eta, "About 2 s");
    p.remaining_work = None;
    assert!(copy(&p, &NumberLocale::default()).eta.is_empty());
}
#[test]
fn move_estimates_outstanding_proofs_after_destination_publication() {
    for policy in [
        TransferVerification::FullSha256,
        TransferVerification::SizeAndSamples,
    ] {
        let mut p = transfer(TransferIntent::Move, policy);
        let readback = match policy {
            TransferVerification::FullSha256 => 1_000_000,
            TransferVerification::SizeAndSamples => 196_608,
        };
        Arc::make_mut(&mut p.throughput).verification_per_second = readback as f64;
        p.completed_bytes = 1_000_000;
        p.files_completed = 1;
        p.remaining_work = Some(TransferRemainingWork {
            write_bytes: 0,
            verification_bytes: 4 * readback,
        });
        assert_eq!(copy(&p, &NumberLocale::default()).eta, "About 4 s");
        p.remaining_work.as_mut().unwrap().verification_bytes = readback / 2;
        assert_eq!(copy(&p, &NumberLocale::default()).eta, "About 1 s");
        // Retried finalization repeats the complete source/destination After pass.
        p.validation_bytes = 9_000_000;
        p.remaining_work.as_mut().unwrap().verification_bytes = 2 * readback;
        assert_eq!(copy(&p, &NumberLocale::default()).eta, "About 2 s");
        assert_eq!(
            copy(&p, &NumberLocale::default()).remaining,
            "Items remaining: 1"
        );
    }
}
#[test]
fn unavailable_work_pause_errors_and_nonfinite_rates_never_fabricate_eta() {
    let mut p = transfer(TransferIntent::Move, TransferVerification::SizeAndSamples);
    p.remaining_work = Some(TransferRemainingWork {
        write_bytes: 10,
        verification_bytes: 20,
    });
    for state in [
        CopyOperationState::WaitingForDecision,
        CopyOperationState::CancelRequested,
        CopyOperationState::Completed,
    ] {
        p.state = state;
        assert!(copy(&p, &NumberLocale::default()).eta.is_empty());
    }
    p.state = CopyOperationState::Running;
    p.paused = true;
    assert!(copy(&p, &NumberLocale::default()).eta.is_empty());
    p.paused = false;
    p.skipped = 1;
    assert!(copy(&p, &NumberLocale::default()).eta.is_empty());
    p.skipped = 0;
    p.failed = 1;
    assert!(copy(&p, &NumberLocale::default()).eta.is_empty());
    p.failed = 0;
    for rate in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        Arc::make_mut(&mut p.throughput).verification_per_second = rate;
        assert!(copy(&p, &NumberLocale::default()).eta.is_empty());
    }
}
#[test]
fn native_root_scope_keeps_measured_item_eta_without_a_byte_budget() {
    let mut p = transfer(TransferIntent::Move, TransferVerification::SizeAndSamples);
    p.discovery.scope = DiscoveryScope::SelectedRoots;
    p.discovery.files = 8;
    p.completed_roots = 2;
    Arc::make_mut(&mut p.throughput).items_per_second = 2.0;
    assert!(p.remaining_work.is_none());
    assert_eq!(copy(&p, &NumberLocale::default()).eta, "About 3 s");
}
