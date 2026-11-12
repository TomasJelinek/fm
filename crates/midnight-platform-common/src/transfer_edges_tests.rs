use super::*;
use midnight_core::{TRANSFER_SAMPLE_BYTES, TransferVerification};

fn sampled(size: usize, corrupt: Option<usize>) -> VerifiedFileCopy {
    let (old, _) = copy((0..size).map(|n| (n % 251) as u8).collect(), false);
    VerifiedFileCopy::new_with_verification(
        old.source,
        Box::new(Stage {
            corrupt_offset: corrupt,
            ..Stage::default()
        }),
        TransferVerification::SizeAndSamples,
    )
    .unwrap()
}

fn finish(job: &mut VerifiedFileCopy) -> Result<(), FsError> {
    while job.phase() != CopyPhase::ReadyToPublish {
        job.step(&Cancellation::default())?;
    }
    Ok(())
}

#[test]
fn quick_readback_is_bounded_and_covers_all_small_files_including_empty() {
    for size in [
        0, 1, 65535, 65536, 65537, 131071, 131072, 131073, 196607, 196608, 196609, 1_000_000,
    ] {
        let mut job = sampled(size, None);
        finish(&mut job).unwrap();
        assert_eq!(job.copied_bytes(), size as u64);
        assert_eq!(
            job.verified_bytes(),
            size.min(3 * TRANSFER_SAMPLE_BYTES) as u64
        );
        job.publish(OsStr::new("target"), &Cancellation::default())
            .unwrap();
    }
}

#[test]
fn quick_mode_rejects_first_last_boundary_and_small_file_corruption() {
    for (size, offset) in [
        (1, 0),
        (131072, 65536),
        (1_000_000, 0),
        (1_000_000, 65535),
        (1_000_000, 500000),
        (196608, 100000),
        (1_000_000, 467232),
        (1_000_000, 532767),
        (1_000_000, 1_000_000 - 65536),
        (1_000_000, 999999),
    ] {
        let mut job = sampled(size, Some(offset));
        assert!(finish(&mut job).is_err(), "size={size}, offset={offset}");
        assert!(!job.can_retry(), "integrity mismatch is terminal");
        assert!(
            job.publish(OsStr::new("target"), &Cancellation::default())
                .is_err()
        );
    }
}

#[test]
fn quick_mode_explicitly_does_not_claim_integrity_between_samples() {
    let mut job = sampled(1_000_000, Some(250_000));
    finish(&mut job).unwrap();
    assert_eq!(job.verified_bytes(), 196608);
    let (mut full, _) = copy(vec![42; 1_000_000], false);
    full.stage = Box::new(Stage {
        corrupt_offset: Some(250_000),
        ..Stage::default()
    });
    assert!(finish(&mut full).is_err());
}

#[test]
fn quick_move_proof_reads_only_edges_and_still_checks_final_identity() {
    let cancel = Cancellation::default();
    let mut job = sampled(1_000_000, None);
    finish(&mut job).unwrap();
    job.publish(OsStr::new("target"), &cancel).unwrap();
    let mut proof = job.into_published_proof(&cancel).unwrap();
    proof.begin_validation(&cancel).unwrap();
    let mut buffer = vec![0; 1001];
    while proof.phase() != FileProofPhase::Complete {
        proof.validation_step(&mut buffer, &cancel).unwrap();
    }
    assert_eq!(proof.validation_bytes(), 6 * TRANSFER_SAMPLE_BYTES as u64);
    proof.revalidate_finalization(&cancel).unwrap();
}

#[test]
fn quick_mode_never_accepts_truncation_sync_failure_changed_source_or_cancellation() {
    for stage in [
        Stage {
            truncate_on_sync: true,
            ..Stage::default()
        },
        Stage {
            fail_sync: true,
            ..Stage::default()
        },
    ] {
        let mut job = sampled(1_000_000, None);
        job.stage = Box::new(stage);
        assert!(finish(&mut job).is_err());
        assert!(
            job.publish(OsStr::new("target"), &Cancellation::default())
                .is_err()
        );
    }
    let (old, changed) = copy(vec![42; 500000], false);
    let mut job = VerifiedFileCopy::new_with_verification(
        old.source,
        old.stage,
        TransferVerification::SizeAndSamples,
    )
    .unwrap();
    job.step(&Cancellation::default()).unwrap();
    *changed.lock().unwrap() = true;
    assert!(finish(&mut job).is_err());
    let mut job = sampled(500000, None);
    finish(&mut job).unwrap();
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(job.publish(OsStr::new("target"), &cancel).is_err());
}

#[test]
fn sample_capture_handles_small_unaligned_source_reads_without_losing_reference_bytes() {
    let bytes: Vec<_> = (0..200_007).map(|n| (n % 251) as u8).collect();
    for chunk_size in [1, 7, 65535, 65536, 65537, 100000] {
        let mut evidence = Edges::new(bytes.len() as u64);
        for chunk in bytes.chunks(chunk_size) {
            evidence.update(chunk);
        }
        assert!(evidence.matches(bytes.len() as u64, 0, &bytes[..65536]));
        let middle = (bytes.len() - 65536) / 2;
        assert!(evidence.matches(bytes.len() as u64, 65536, &bytes[middle..middle + 65536]));
        assert!(evidence.matches(bytes.len() as u64, 131072, &bytes[bytes.len() - 65536..]));
    }
}
