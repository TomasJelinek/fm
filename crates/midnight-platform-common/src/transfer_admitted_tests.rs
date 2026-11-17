use super::*;
use midnight_work_budget::{Admission, Gate};
use std::{sync::mpsc, thread, time::Duration};

struct Deadline {
    done: mpsc::Sender<()>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Deadline {
    fn new(cancel: &Cancellation) -> Self {
        let (done, receiver) = mpsc::channel();
        let cancel = cancel.clone();
        Self {
            done,
            worker: Some(thread::spawn(move || {
                if receiver.recv_timeout(Duration::from_secs(4)).is_err() {
                    cancel.cancel();
                }
            })),
        }
    }
}
impl Drop for Deadline {
    fn drop(&mut self) {
        let _ = self.done.send(());
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

struct InterruptedReadback {
    inner: Stage,
    failed: bool,
    reads: Arc<Mutex<Vec<u64>>>,
}
impl Read for InterruptedReadback {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let position = self.inner.data.position();
        let call = {
            let mut reads = self.reads.lock().unwrap();
            let call = reads.len();
            reads.push(position);
            call
        };
        if call == 1 && !self.failed {
            self.failed = true;
            self.inner.data.set_position(position + 3);
            return Err(io::ErrorKind::StorageFull.into());
        }
        let count = bytes.len().min(65536);
        self.inner.read(&mut bytes[..count])
    }
}
impl Write for InterruptedReadback {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.inner.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
impl Seek for InterruptedReadback {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.inner.seek(position)
    }
}
impl StagedTransferFile for InterruptedReadback {
    fn sync(&mut self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.sync(cancel)
    }
    fn len(&self) -> Result<u64, FsError> {
        self.inner.len()
    }
    fn cleanup(&mut self) -> Result<(), FsError> {
        self.inner.cleanup()
    }
    fn publish_no_replace(
        &mut self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<TransferPublication, FsError> {
        self.inner.publish_no_replace(name, cancel)
    }
    fn revalidate_publication(&self, cancel: &Cancellation) -> Result<TransferIdentity, FsError> {
        self.inner.revalidate_publication(cancel)
    }
    fn rewind_for_validation(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn midnight_core::TransferReadGuard>, FsError> {
        self.inner.rewind_for_validation(cancel)
    }
}

#[test]
fn configured_limit_one_copy_retry_and_move_proof_preserve_full_and_sampled_evidence() {
    let gate = Arc::new(Gate::new(1));
    let permit = gate.acquire(Admission::Shared, || false).unwrap();
    crate::transfer_io::with_transfer_io_permit_from(&gate, &permit, || {
        for policy in [
            TransferVerification::default(),
            TransferVerification::SizeAndSamples,
        ] {
            let (old, _) = copy(vec![42; 600_007], false);
            let reads = Arc::new(Mutex::new(Vec::new()));
            let mut job = VerifiedFileCopy::new_with_verification(
                old.source,
                Box::new(InterruptedReadback {
                    inner: Stage::default(),
                    failed: false,
                    reads: reads.clone(),
                }),
                policy,
            )
            .unwrap();
            let cancel = Cancellation::default();
            let _deadline = Deadline::new(&cancel);
            while job.phase() != CopyPhase::Verifying {
                job.step_admitted(&cancel, &permit).unwrap();
            }
            job.step_admitted(&cancel, &permit).unwrap();
            assert_eq!(job.verified_bytes(), 65536);
            let written = job.copied_bytes();
            assert!(job.step_admitted(&cancel, &permit).is_err());
            assert_eq!(job.verified_bytes(), 65536);
            assert_eq!(job.copied_bytes(), written);
            job.retry_admitted(&cancel, &permit).unwrap();
            assert_eq!(job.verified_bytes(), 65536);
            while job.phase() != CopyPhase::ReadyToPublish {
                job.step_admitted(&cancel, &permit).unwrap();
            }
            assert_eq!(job.copied_bytes(), 600_007);
            assert_eq!(job.verified_bytes(), policy.readback_bytes(600_007));
            assert_eq!(
                reads
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|offset| **offset == 0)
                    .count(),
                1
            );
            assert!(matches!(
                job.publish_admitted(OsStr::new("target"), &cancel, &permit)
                    .unwrap(),
                TransferPublication::Published(_)
            ));
            let mut proof = job.into_published_proof_admitted(&cancel, &permit).unwrap();
            proof.begin_validation_admitted(&cancel, &permit).unwrap();
            let mut buffer = vec![0; 65536];
            while proof.phase() != FileProofPhase::Complete {
                proof
                    .validation_step_admitted(&mut buffer, &cancel, &permit)
                    .unwrap();
            }
            assert_eq!(proof.validation_bytes(), 2 * policy.readback_bytes(600_007));
            proof
                .revalidate_publication_admitted(&cancel, &permit)
                .unwrap();
            proof
                .revalidate_finalization_admitted(&cancel, &permit)
                .unwrap();
        }
    })
    .unwrap();
    assert_eq!(gate.snapshot().active_shared, 1);
    assert_eq!(gate.snapshot().queued, 0);
}

#[test]
fn configured_limit_one_replacement_keeps_the_verified_stage_and_exact_outcome() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let gate = Arc::new(Gate::new(1));
    let permit = gate.acquire(Admission::Shared, || false).unwrap();
    crate::transfer_io::with_transfer_io_permit_from(&gate, &permit, || {
        let (mut job, _) = copy(b"retained bytes".to_vec(), false);
        let preparations = Arc::new(AtomicUsize::new(0));
        let publications = Arc::new(AtomicUsize::new(0));
        job.stage = Box::new(Stage {
            preparations: Some(preparations.clone()),
            replacement: Some((
                publications.clone(),
                TransferPublication::Replaced {
                    path: "target".into(),
                    recovery: midnight_core::TransferRecovery {
                        path: "recovery".into(),
                        identity: None,
                    },
                },
            )),
            ..Stage::default()
        });
        let target = Arc::new(Target(job.source.entry().clone()));
        let cancel = Cancellation::default();
        let _deadline = Deadline::new(&cancel);
        while job.phase() != CopyPhase::ReadyToPublish {
            job.step_admitted(&cancel, &permit).unwrap();
        }
        let verified = job.verified_bytes();
        job.prepare_replacement_admitted(
            OsStr::new("target"),
            target,
            TransferReplacementPolicy::RecoverableCurrentEntry,
            &cancel,
            &permit,
        )
        .unwrap();
        assert!(matches!(
            job.publish_replace_admitted(OsStr::new("target"), &cancel, &permit)
                .unwrap(),
            TransferPublication::Replaced { .. }
        ));
        assert_eq!(job.verified_bytes(), verified);
        assert_eq!(preparations.load(Ordering::SeqCst), 1);
        assert_eq!(publications.load(Ordering::SeqCst), 1);
        job.cleanup_admitted(&permit).unwrap();
    })
    .unwrap();
    assert_eq!(gate.snapshot().active_shared, 1);
}

#[test]
fn rejected_admitted_witness_does_not_advance_or_clean_the_copy() {
    let wrong = Arc::new(Gate::new(1))
        .try_acquire(Admission::Shared)
        .unwrap();
    let (mut job, _) = copy(b"bytes".to_vec(), false);
    assert!(job.step_admitted(&Cancellation::default(), &wrong).is_err());
    assert!(job.cleanup_admitted(&wrong).is_err());
    assert_eq!(job.phase(), CopyPhase::Copying);
    assert_eq!(job.copied_bytes(), 0);
    assert_eq!(job.verified_bytes(), 0);
}
