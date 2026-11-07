use super::*;
use midnight_core::TransferModifiedTimePin;
use std::time::{Duration, SystemTime};

pub(super) struct State {
    pub(super) events: Vec<&'static str>,
    pub(super) actual: SystemTime,
    pub(super) failure: &'static str,
    pub(super) source_changed: bool,
}
struct Pin(Arc<Mutex<State>>);
fn expected() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(12345)
}
impl TransferModifiedTimePin for Pin {
    fn modified(&self) -> SystemTime {
        expected()
    }
    fn current_modified(&self, _: &Cancellation) -> Result<SystemTime, FsError> {
        Ok(if self.0.lock().unwrap().source_changed {
            SystemTime::UNIX_EPOCH
        } else {
            expected()
        })
    }
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        if self.0.lock().unwrap().source_changed {
            Err(failure(Path::new("source"), "source metadata changed"))
        } else {
            Ok(())
        }
    }
}
fn required(fail: &'static str) -> (VerifiedFileCopy, Arc<Mutex<State>>) {
    let state = Arc::new(Mutex::new(State {
        events: vec![],
        actual: SystemTime::UNIX_EPOCH,
        failure: fail,
        source_changed: false,
    }));
    let (mut copy, _) = copy(
        b"required metadata and independently checked contents".to_vec(),
        false,
    );
    copy.stage = Box::new(Stage {
        metadata: Some(state.clone()),
        ..Stage::default()
    });
    copy.modified_time = Some(Box::new(Pin(state.clone())));
    (copy, state)
}
#[test]
fn required_metadata_runs_after_checksum_readback_then_syncs_and_reads_exact_time() {
    let (mut copy, state) = required("");
    while copy.phase() != CopyPhase::ReadyToPublish {
        copy.step(&Cancellation::default()).unwrap();
    }
    let state = state.lock().unwrap();
    assert_eq!(state.actual, expected());
    let set = state.events.iter().position(|v| *v == "set-time").unwrap();
    assert!(state.events[..set].contains(&"data-read"));
    assert_eq!(
        &state.events[set..],
        &["set-time", "metadata-sync", "read-time"]
    );
}
#[test]
fn metadata_set_sync_readback_and_cancel_failures_cannot_publish_or_create_proof() {
    for fail in ["set", "sync", "readback", "cancel"] {
        let (mut copy, _) = required(fail);
        let cancel = Cancellation::default();
        let mut failed = false;
        for _ in 0..30 {
            if copy.step(&cancel).is_err() {
                failed = true;
                break;
            }
            if copy.phase() == CopyPhase::ReadyToPublish {
                break;
            }
        }
        assert!(failed, "{fail}");
        assert!(
            copy.publish(OsStr::new("target"), &cancel).is_err(),
            "{fail}"
        );
        assert!(copy.into_published_proof(&cancel).is_err(), "{fail}");
    }
}
#[test]
fn late_source_metadata_edit_blocks_publication_even_after_required_readback() {
    let (mut copy, state) = required("");
    let cancel = Cancellation::default();
    while copy.phase() != CopyPhase::ReadyToPublish {
        copy.step(&cancel).unwrap();
    }
    state.lock().unwrap().source_changed = true;
    assert!(copy.publish(OsStr::new("target"), &cancel).is_err());
    assert_eq!(copy.phase(), CopyPhase::Failed);
}
#[test]
fn modified_published_target_is_not_corrected_and_blocks_move_proof() {
    let (mut copy, state) = required("");
    let cancel = Cancellation::default();
    while copy.phase() != CopyPhase::ReadyToPublish {
        copy.step(&cancel).unwrap();
    }
    copy.publish(OsStr::new("target"), &cancel).unwrap();
    let mut proof = copy.into_published_proof(&cancel).unwrap();
    state.lock().unwrap().actual = SystemTime::UNIX_EPOCH;
    let result = proof.begin_validation(&cancel).and_then(|()| {
        for _ in 0..30 {
            if proof.validation_step(&mut [0; 64], &cancel)? == FileProofPhase::Complete {
                return Ok(());
            }
        }
        panic!("proof did not terminate")
    });
    assert!(result.is_err());
    assert_eq!(state.lock().unwrap().actual, SystemTime::UNIX_EPOCH);
}
