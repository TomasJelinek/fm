//! Remaining work is cross-checked against real content I/O through publication/proofs.
use super::*;

#[test]
fn sampled_copy_budget_tracks_multiple_files_without_inventing_full_readback() {
    let temp = fixture();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(source.join("large"), vec![7; 400_000]).unwrap();
    fs::write(source.join("small"), vec![8; 100]).unwrap();
    fs::write(source.join("empty"), []).unwrap();
    let mut input = request(1, &[source], &destination);
    input.verification = TransferVerification::SizeAndSamples;
    let status = Arc::new(Mutex::new(initial(&input.clone().into())));
    let service = midnight_platform::LocalFileSystem::new(None);
    let mut job =
        worker::Job::prepare(input, &service, Cancellation::default(), status.clone()).unwrap();
    job.prepare_discovery();
    assert_eq!(
        snapshot(&status).remaining_work,
        Some(TransferRemainingWork {
            write_bytes: 400_100,
            verification_bytes: 196_708,
        })
    );
    for _ in 0..150 {
        let step = job.step();
        let p = snapshot(&status);
        let remaining = p.remaining_work.expect("known clean recursive Copy");
        assert_eq!(remaining.write_bytes + p.copied_bytes, 400_100);
        assert_eq!(remaining.verification_bytes + p.verified_bytes, 196_708);
        match step {
            worker::Step::Continue => {}
            worker::Step::Done => {
                let done = job.finish();
                assert_eq!(done.state, CopyOperationState::Completed);
                assert_eq!(done.remaining_work, Some(TransferRemainingWork::default()));
                assert_eq!(done.files_completed, 3);
                return;
            }
            _ => panic!("unexpected Copy decision: {p:?}"),
        }
    }
    panic!("Copy did not complete");
}

#[test]
fn cross_volume_move_eta_keeps_both_proof_passes_after_publication() {
    for (policy, readback) in [
        (TransferVerification::FullSha256, 400_000),
        (TransferVerification::SizeAndSamples, 196_608),
    ] {
        let (_temp, root) = moves::protected_fixture();
        let source = root.join("source");
        let destination = root.join("destination");
        fs::write(&source, vec![9; 400_000]).unwrap();
        fs::create_dir(&destination).unwrap();
        let mut input = request(1, std::slice::from_ref(&source), &destination);
        input.verification = policy;
        let status = Arc::new(Mutex::new(initial(&CopyInput::move_paths(input.clone()))));
        let service = moves::CrossDeviceService(midnight_platform::LocalFileSystem::new(None));
        let mut job =
            worker::Job::prepare_move(input, &service, Cancellation::default(), status.clone())
                .unwrap();
        job.discover_cross_device_fixture();
        assert_eq!(
            snapshot(&status).remaining_work,
            Some(TransferRemainingWork {
                write_bytes: 400_000,
                verification_bytes: 5 * readback,
            })
        );
        let mut before = false;
        let mut after = false;
        let mut published_with_proofs = false;
        let mut done = None;
        for _ in 0..150 {
            let step = job.step();
            let p = snapshot(&status);
            let remaining = p.remaining_work.expect("known clean cross-volume Move");
            assert_eq!(remaining.write_bytes + p.copied_bytes, 400_000);
            assert_eq!(
                remaining.verification_bytes + p.verified_bytes + p.validation_bytes,
                5 * readback,
                "policy={policy:?}, phase={:?}",
                p.phase
            );
            if p.files_completed == 1 && p.validation_bytes == 0 {
                published_with_proofs = true;
                assert_eq!(remaining.verification_bytes, 4 * readback);
            }
            before |= p.phase == CopyOperationPhase::ValidatingBeforeMove;
            after |= p.phase == CopyOperationPhase::ValidatingAfterMove;
            match step {
                worker::Step::Continue => {}
                worker::Step::Done => {
                    done = Some(job.finish());
                    break;
                }
                _ => panic!("unexpected Move decision: {p:?}"),
            }
        }
        let done = done.expect("bounded Move completion");
        assert_eq!(done.state, CopyOperationState::Completed);
        assert_eq!(done.remaining_work, Some(TransferRemainingWork::default()));
        assert!(before && after && published_with_proofs);
        assert_eq!(done.source_files_removed, 1);
        assert_eq!(done.validation_bytes, 4 * readback);
        assert!(!source.exists());
    }
}
