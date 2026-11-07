use super::*;
use midnight_core::{EntryKind, EntryMetadata, MutationItem, TransferIdentity};
use std::{
    io::{self, Cursor, Read, Seek, SeekFrom, Write},
    sync::{Arc, Mutex},
};
#[path = "transfer_metadata_tests.rs"]
mod metadata;

struct Source {
    entry: TransferEntry,
    data: Cursor<Vec<u8>>,
    changed: Arc<Mutex<bool>>,
}
impl TransferSource for Source {
    fn rewind_for_validation(
        &mut self,
        _: &Cancellation,
    ) -> Result<Box<dyn midnight_core::TransferReadGuard>, FsError> {
        self.data.set_position(0);
        Ok(Box::new(ReadGuard(self.changed.clone())))
    }
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        if *self.changed.lock().unwrap() {
            Err(failure(&self.entry.item.path, "source changed"))
        } else {
            Ok(())
        }
    }
    fn read_chunk(&mut self, data: &mut [u8], _: &Cancellation) -> Result<usize, FsError> {
        Ok(self.data.read(data).unwrap())
    }
    fn next_child(&mut self, _: &Cancellation) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        unreachable!()
    }
}
#[derive(Default)]
struct Stage {
    metadata: Option<Arc<Mutex<metadata::State>>>,
    data: Cursor<Vec<u8>>,
    corrupt_on_sync: bool,
    truncate_on_sync: bool,
    fail_sync: bool,
    replacement: Option<(Arc<std::sync::atomic::AtomicUsize>, TransferPublication)>,
    preparations: Option<Arc<std::sync::atomic::AtomicUsize>>,
    changed: Option<Arc<Mutex<bool>>>,
}
impl Read for Stage {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        if let Some(state) = &self.metadata {
            state.lock().unwrap().events.push("data-read");
        }
        self.data.read(b)
    }
}
impl Write for Stage {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.data.write(&b[..b.len().min(7)])
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Seek for Stage {
    fn seek(&mut self, p: SeekFrom) -> io::Result<u64> {
        self.data.seek(p)
    }
}
impl StagedTransferFile for Stage {
    fn set_modified_time(
        &mut self,
        expected: std::time::SystemTime,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        let state = self.metadata.as_ref().expect("metadata enabled");
        let mut state = state.lock().unwrap();
        state.events.push("set-time");
        if state.failure == "set" {
            return Err(failure(Path::new("stage"), "injected metadata set error"));
        }
        state.actual = expected;
        if state.failure == "cancel" {
            cancel.cancel();
        }
        Ok(())
    }
    fn modified_time(&self, _: &Cancellation) -> Result<std::time::SystemTime, FsError> {
        let mut state = self
            .metadata
            .as_ref()
            .expect("metadata enabled")
            .lock()
            .unwrap();
        state.events.push("read-time");
        Ok(if state.failure == "readback" {
            std::time::SystemTime::UNIX_EPOCH
        } else {
            state.actual
        })
    }
    fn rewind_for_validation(
        &mut self,
        _: &Cancellation,
    ) -> Result<Box<dyn midnight_core::TransferReadGuard>, FsError> {
        self.data.set_position(0);
        Ok(Box::new(ReadGuard(
            self.changed.clone().unwrap_or_default(),
        )))
    }
    fn revalidate_publication(&self, _: &Cancellation) -> Result<TransferIdentity, FsError> {
        if self
            .changed
            .as_ref()
            .is_some_and(|value| *value.lock().unwrap())
        {
            return Err(failure(Path::new("stage"), "published name changed"));
        }
        Ok(TransferIdentity {
            device: 1,
            object: [9; 16],
        })
    }
    fn sync(&mut self, _: &Cancellation) -> Result<(), FsError> {
        if let Some(state) = &self.metadata {
            let mut state = state.lock().unwrap();
            let metadata = state.events.contains(&"set-time");
            state.events.push(if metadata {
                "metadata-sync"
            } else {
                "data-sync"
            });
            if metadata && state.failure == "sync" {
                return Err(failure(
                    Path::new("stage"),
                    "injected metadata sync failure",
                ));
            }
        }
        if self.fail_sync {
            return Err(io_error(
                Path::new("stage"),
                io::Error::other("injected sync failure"),
            ));
        }
        if self.corrupt_on_sync && !self.data.get_ref().is_empty() {
            self.data.get_mut()[0] ^= 1;
        }
        if self.truncate_on_sync {
            self.data.get_mut().pop();
        }
        Ok(())
    }
    fn len(&self) -> Result<u64, FsError> {
        Ok(self.data.get_ref().len() as u64)
    }
    fn publish_no_replace(
        &mut self,
        name: &OsStr,
        _: &Cancellation,
    ) -> Result<TransferPublication, FsError> {
        Ok(TransferPublication::Published(name.into()))
    }
    fn cleanup(&mut self) -> Result<(), FsError> {
        Ok(())
    }
    fn publish_replace(
        &mut self,
        _: &OsStr,
        _: &Cancellation,
    ) -> Result<TransferPublication, FsError> {
        let (calls, result) = self.replacement.as_ref().expect("configured replacement");
        calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(result.clone())
    }
    fn prepare_replacement(
        &mut self,
        _: &OsStr,
        _: Arc<dyn TransferTarget>,
        _: TransferReplacementPolicy,
        _: &Cancellation,
    ) -> Result<(), FsError> {
        self.preparations
            .as_ref()
            .expect("configured preparation")
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

struct Target(TransferEntry);
struct ReadGuard(Arc<Mutex<bool>>);
impl midnight_core::TransferReadGuard for ReadGuard {
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        if *self.0.lock().unwrap() {
            Err(failure(Path::new("guard"), "revision changed"))
        } else {
            Ok(())
        }
    }
}
impl TransferTarget for Target {
    fn entry(&self) -> &TransferEntry {
        &self.0
    }
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        Ok(())
    }
}

#[test]
fn replacement_preparation_reuses_verified_stage_and_rejects_changed_source() {
    let (mut job, changed) = copy(b"verified bytes".to_vec(), false);
    let preparations = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let target = Arc::new(Target(job.source.entry().clone()));
    job.stage = Box::new(Stage {
        preparations: Some(preparations.clone()),
        ..Stage::default()
    });
    let cancel = Cancellation::default();
    while job.phase() != CopyPhase::ReadyToPublish {
        job.step(&cancel).unwrap();
    }
    job.prepare_replacement(
        OsStr::new("target"),
        target.clone(),
        TransferReplacementPolicy::RecoverableCurrentEntry,
        &cancel,
    )
    .unwrap();
    assert_eq!(job.phase(), CopyPhase::ReadyToPublish);
    assert_eq!(job.copied_bytes(), 14);
    assert_eq!(job.verified_bytes(), 14);
    assert_eq!(preparations.load(std::sync::atomic::Ordering::SeqCst), 1);
    *changed.lock().unwrap() = true;
    assert!(
        job.prepare_replacement(
            OsStr::new("target"),
            target,
            TransferReplacementPolicy::RecoverableCurrentEntry,
            &cancel
        )
        .is_err()
    );
    assert_eq!(job.phase(), CopyPhase::Failed);
    assert_eq!(preparations.load(std::sync::atomic::Ordering::SeqCst), 1);
}
pub(super) fn copy(data: Vec<u8>, corrupt: bool) -> (VerifiedFileCopy, Arc<Mutex<bool>>) {
    let changed = Arc::new(Mutex::new(false));
    let entry = TransferEntry {
        item: MutationItem {
            path: "source".into(),
            kind: EntryKind::File,
            metadata: EntryMetadata {
                size: Some(data.len() as u64),
                ..EntryMetadata::default()
            },
        },
        identity: TransferIdentity {
            device: 1,
            object: [1; 16],
        },
    };
    (
        VerifiedFileCopy::new(
            Box::new(Source {
                entry,
                data: Cursor::new(data),
                changed: changed.clone(),
            }),
            Box::new(Stage {
                corrupt_on_sync: corrupt,
                ..Stage::default()
            }),
        )
        .unwrap(),
        changed,
    )
}

pub(super) fn copy_with_outcome(outcome: TransferPublication) -> VerifiedFileCopy {
    let (mut copy, _) = copy(b"bytes".to_vec(), false);
    copy.stage = Box::new(Stage {
        replacement: Some((Arc::default(), outcome)),
        ..Stage::default()
    });
    copy
}

#[test]
fn verified_copy_requires_independent_readback_and_handles_short_writes() {
    let (mut job, _) = copy(vec![42; 600_000], false);
    let cancel = Cancellation::default();
    assert!(job.publish(OsStr::new("target"), &cancel).is_err());
    while job.phase() != CopyPhase::ReadyToPublish {
        let before = job.copied_bytes();
        job.step(&cancel).unwrap();
        assert!(job.copied_bytes() - before <= 256 * 1024);
    }
    assert_eq!(job.copied_bytes(), 600_000);
    assert_eq!(job.verified_bytes(), 600_000);
    assert!(matches!(
        job.publish(OsStr::new("target"), &cancel).unwrap(),
        TransferPublication::Published(_)
    ));
}

#[test]
fn same_length_target_corruption_prevents_publication() {
    let (mut job, _) = copy(b"original bytes".to_vec(), true);
    let cancel = Cancellation::default();
    let error = loop {
        if let Err(error) = job.step(&cancel) {
            break error;
        }
    };
    assert_eq!(error.kind, io::ErrorKind::InvalidData);
    assert!(job.publish(OsStr::new("target"), &cancel).is_err());
}

#[test]
fn source_change_while_waiting_to_publish_is_rejected() {
    let (mut job, changed) = copy(b"original bytes".to_vec(), false);
    let cancel = Cancellation::default();
    while job.phase() != CopyPhase::ReadyToPublish {
        job.step(&cancel).unwrap();
    }
    *changed.lock().unwrap() = true;
    assert!(job.publish(OsStr::new("target"), &cancel).is_err());
}

#[test]
fn cancellation_stops_between_blocks_and_empty_files_are_verified() {
    let (mut job, _) = copy(vec![0; 600_000], false);
    let cancel = Cancellation::default();
    job.step(&cancel).unwrap();
    cancel.cancel();
    assert_eq!(
        job.step(&cancel).unwrap_err().kind,
        io::ErrorKind::Interrupted
    );
    let (mut empty, _) = copy(vec![], false);
    while empty.phase() != CopyPhase::ReadyToPublish {
        empty.step(&Cancellation::default()).unwrap();
    }
    assert_eq!(empty.verified_bytes(), 0);
}

#[test]
fn sync_failure_or_target_truncation_can_never_reach_publication() {
    for (truncate_on_sync, fail_sync) in [(true, false), (false, true)] {
        let (mut job, _) = copy(b"original bytes".to_vec(), false);
        job.stage = Box::new(Stage {
            truncate_on_sync,
            fail_sync,
            ..Stage::default()
        });
        let cancel = Cancellation::default();
        loop {
            if job.step(&cancel).is_err() {
                break;
            }
            assert_ne!(job.phase(), CopyPhase::ReadyToPublish);
        }
        assert_eq!(job.phase(), CopyPhase::Failed);
        assert!(job.publish(OsStr::new("target"), &cancel).is_err());
    }
}

fn replacement_result() -> TransferPublication {
    TransferPublication::Replaced {
        path: "target".into(),
        recovery: midnight_core::TransferRecovery {
            path: "recovery/original".into(),
            identity: Some(TransferIdentity {
                device: 1,
                object: [2; 16],
            }),
        },
    }
}

#[test]
fn replacement_requires_full_independent_readback_and_retains_recovery_receipt() {
    let (mut job, _) = copy(vec![42; 600_000], false);
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    job.stage = Box::new(Stage {
        replacement: Some((calls.clone(), replacement_result())),
        ..Stage::default()
    });
    let cancel = Cancellation::default();
    while job.phase() != CopyPhase::ReadyToPublish {
        assert!(job.publish_replace(OsStr::new("target"), &cancel).is_err());
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        job.step(&cancel).unwrap();
    }
    assert_eq!(job.verified_bytes(), 600_000);
    let result = job.publish_replace(OsStr::new("target"), &cancel).unwrap();
    assert!(
        matches!(result, TransferPublication::Replaced { recovery, .. }
        if recovery.path == Path::new("recovery/original"))
    );
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(job.publish_replace(OsStr::new("target"), &cancel).is_err());
}

#[test]
fn replacement_is_blocked_by_corruption_cancellation_and_source_change() {
    for scenario in 0..3 {
        let (mut job, changed) = copy(b"verified replacement".to_vec(), false);
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        job.stage = Box::new(Stage {
            corrupt_on_sync: scenario == 0,
            replacement: Some((calls.clone(), replacement_result())),
            ..Stage::default()
        });
        let cancel = Cancellation::default();
        while job.phase() != CopyPhase::ReadyToPublish {
            if job.step(&cancel).is_err() {
                break;
            }
        }
        match scenario {
            1 => cancel.cancel(),
            2 => *changed.lock().unwrap() = true,
            _ => {}
        }
        assert!(job.publish_replace(OsStr::new("target"), &cancel).is_err());
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }
}

#[test]
fn partial_commit_is_terminal_and_preserves_recovery_facts() {
    for published in [None, Some("target".into())] {
        let (mut job, _) = copy(b"verified replacement".to_vec(), false);
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let recovery = midnight_core::TransferRecovery {
            path: "recovery/unexpected".into(),
            identity: Some(TransferIdentity {
                device: 1,
                object: [3; 16],
            }),
        };
        job.stage = Box::new(Stage {
            replacement: Some((
                calls.clone(),
                TransferPublication::RecoveryRequired {
                    published: published.clone(),
                    recovery: vec![recovery],
                    error: failure(Path::new("target"), "concurrent destination change"),
                },
            )),
            ..Stage::default()
        });
        let cancel = Cancellation::default();
        while job.phase() != CopyPhase::ReadyToPublish {
            job.step(&cancel).unwrap();
        }
        let result = job.publish_replace(OsStr::new("target"), &cancel).unwrap();
        assert!(matches!(result, TransferPublication::RecoveryRequired {
            published: actual, recovery, ..
        } if actual == published && recovery[0].path == Path::new("recovery/unexpected")));
        assert_eq!(job.phase(), CopyPhase::RecoveryRequired);
        assert!(job.publish(OsStr::new("another"), &cancel).is_err());
        assert!(job.publish_replace(OsStr::new("another"), &cancel).is_err());
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
