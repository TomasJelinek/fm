use super::*;
use midnight_core::{EntryKind, EntryMetadata, MutationItem, TransferIdentity};
use std::{
    io::{self, Cursor, Read, Seek, SeekFrom, Write},
    sync::{Arc, Mutex},
};

struct Source {
    entry: TransferEntry,
    data: Cursor<Vec<u8>>,
    changed: Arc<Mutex<bool>>,
}
impl TransferSource for Source {
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
struct Stage {
    data: Cursor<Vec<u8>>,
    corrupt_on_sync: bool,
    truncate_on_sync: bool,
    fail_sync: bool,
}
impl Read for Stage {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
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
    fn sync(&mut self, _: &Cancellation) -> Result<(), FsError> {
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
}
fn copy(data: Vec<u8>, corrupt: bool) -> (VerifiedFileCopy, Arc<Mutex<bool>>) {
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
                data: Cursor::new(Vec::new()),
                corrupt_on_sync: corrupt,
                truncate_on_sync: false,
                fail_sync: false,
            }),
        )
        .unwrap(),
        changed,
    )
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
            data: Cursor::new(Vec::new()),
            corrupt_on_sync: false,
            truncate_on_sync,
            fail_sync,
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
