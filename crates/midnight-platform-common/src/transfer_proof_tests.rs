use super::*;
use crate::transfer::tests::{copy, copy_with_outcome};

fn published(data: Vec<u8>) -> (VerifiedFileCopy, Arc<std::sync::Mutex<bool>>) {
    let (mut copy, changed) = copy(data, false);
    let cancel = Cancellation::default();
    while copy.phase() != CopyPhase::ReadyToPublish {
        copy.step(&cancel).unwrap();
    }
    copy.publish(OsStr::new("target"), &cancel).unwrap();
    (copy, changed)
}

fn validate(proof: &mut VerifiedFileProof, cancel: &Cancellation) -> Result<(), FsError> {
    proof.begin_validation(cancel)?;
    let mut buffer = vec![0; 256 * 1024];
    while proof.phase() != FileProofPhase::Complete {
        proof.validation_step(&mut buffer, cancel)?;
    }
    Ok(())
}

#[test]
fn retained_proof_rehashes_both_streams_on_each_pass_without_a_private_buffer() {
    for length in [0, 600_000] {
        let (copy, _) = published(vec![42; length]);
        let mut proof = copy.into_published_proof(&Cancellation::default()).unwrap();
        for _ in 0..2 {
            validate(&mut proof, &Cancellation::default()).unwrap();
            assert_eq!(proof.validation_bytes(), 2 * length as u64);
        }
        assert!(std::mem::size_of::<VerifiedFileProof>() < 4096);
    }
}

#[test]
fn no_unpublished_durability_uncertain_or_partial_receipt_authorizes_move() {
    let (unpublished, _) = copy(b"bytes".to_vec(), false);
    assert!(
        unpublished
            .into_published_proof(&Cancellation::default())
            .is_err()
    );
    for outcome in [
        TransferPublication::PublishedDurabilityUnconfirmed {
            path: "target".into(),
            error: failure(Path::new("target"), "sync"),
        },
        TransferPublication::RecoveryRequired {
            published: Some("target".into()),
            recovery: vec![],
            error: failure(Path::new("target"), "partial"),
        },
    ] {
        let mut copy = copy_with_outcome(outcome);
        let cancel = Cancellation::default();
        while copy.phase() != CopyPhase::ReadyToPublish {
            copy.step(&cancel).unwrap();
        }
        copy.publish_replace(OsStr::new("target"), &cancel).unwrap();
        assert!(copy.into_published_proof(&cancel).is_err());
    }
}

#[test]
fn changed_source_destination_binding_and_cancel_fail_terminally() {
    for scenario in 0..4 {
        let (copy, changed) = published(b"verified bytes".to_vec());
        let cancel = Cancellation::default();
        let mut proof = copy.into_published_proof(&cancel).unwrap();
        if scenario == 0 {
            *changed.lock().unwrap() = true;
        }
        if scenario == 1 {
            proof.digest[0] ^= 1;
        }
        if scenario == 2 {
            proof.identity.object[0] ^= 1;
        }
        if scenario == 3 {
            cancel.cancel();
        }
        assert!(validate(&mut proof, &cancel).is_err());
        assert_eq!(proof.phase(), FileProofPhase::Failed);
        assert!(proof.begin_validation(&Cancellation::default()).is_err());
    }
}

#[test]
fn destination_read_corruption_after_a_successful_pass_blocks_the_next_pass() {
    let (copy, _) = published(b"verified bytes".to_vec());
    let cancel = Cancellation::default();
    let mut proof = copy.into_published_proof(&cancel).unwrap();
    validate(&mut proof, &cancel).unwrap();
    proof.stage.seek(SeekFrom::Start(0)).unwrap();
    proof.stage.write_all(b"changed! bytes").unwrap();
    assert!(validate(&mut proof, &cancel).is_err());
    assert_eq!(proof.phase(), FileProofPhase::Failed);
}

#[test]
fn source_revision_change_between_validation_blocks_is_rejected() {
    let (copy, changed) = published(vec![42; 600_000]);
    let cancel = Cancellation::default();
    let mut proof = copy.into_published_proof(&cancel).unwrap();
    proof.begin_validation(&cancel).unwrap();
    let mut buffer = vec![0; 256 * 1024];
    proof.validation_step(&mut buffer, &cancel).unwrap();
    *changed.lock().unwrap() = true;
    while proof.validation_step(&mut buffer, &cancel).is_ok() {}
    assert_eq!(proof.phase(), FileProofPhase::Failed);
}

#[test]
fn completed_hash_pass_keeps_fresh_guards_for_finalization() {
    let (copy, changed) = published(b"verified bytes".to_vec());
    let cancel = Cancellation::default();
    let mut proof = copy.into_published_proof(&cancel).unwrap();
    assert!(
        proof.revalidate_finalization(&cancel).is_err(),
        "publication alone is not a finalization proof"
    );
    validate(&mut proof, &cancel).unwrap();
    proof.revalidate_finalization(&cancel).unwrap();
    *changed.lock().unwrap() = true;
    assert!(
        proof.revalidate_finalization(&cancel).is_err(),
        "a late source edit after EOF must block removal"
    );
}

#[test]
fn sampled_move_retains_guards_and_detects_postpublication_middle_corruption() {
    for scenario in 0..3 {
        let (old, source_changed) = copy(vec![42; 600_000], false);
        let target_changed = Arc::new(std::sync::Mutex::new(false));
        let mut job = VerifiedFileCopy::new_with_verification(
            old.source,
            crate::transfer::tests::stage_with_guard(target_changed.clone()),
            TransferVerification::SizeAndSamples,
        )
        .unwrap();
        let cancel = Cancellation::default();
        while job.phase() != CopyPhase::ReadyToPublish {
            job.step(&cancel).unwrap();
        }
        job.publish(OsStr::new("target"), &cancel).unwrap();
        let mut proof = job.into_published_proof(&cancel).unwrap();
        validate(&mut proof, &cancel).unwrap();
        proof.revalidate_finalization(&cancel).unwrap();
        match scenario {
            0 => *source_changed.lock().unwrap() = true,
            1 => *target_changed.lock().unwrap() = true,
            _ => {
                proof.stage.seek(SeekFrom::Start(300_000)).unwrap();
                proof.stage.write_all(b"changed").unwrap();
                assert!(validate(&mut proof, &cancel).is_err());
                assert_eq!(proof.phase(), FileProofPhase::Failed);
            }
        }
        assert!(proof.revalidate_finalization(&cancel).is_err());
    }
}
