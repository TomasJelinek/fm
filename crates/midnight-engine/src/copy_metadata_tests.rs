use super::*;
use std::{
    fs::{File, FileTimes},
    time::{Duration, SystemTime},
};
fn expected() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::new(123456, 123456700)
}
#[tokio::test]
async fn metadata_wrapper_cannot_downgrade_move_clipboard_to_ordinary_copy() {
    let (_temp, parent) = protected_fixture();
    let source = parent.join("source");
    let destination = parent.join("destination");
    fs::write(&source, b"retained").unwrap();
    fs::create_dir(&destination).unwrap();
    let mut harness = Harness::native();
    harness
        .controller
        .enqueue_metadata(MetadataTransferRequest {
            input: MetadataTransferInput::Clipboard(ClipboardMoveRequest {
                id: OperationId(1),
                clipboard: Arc::new(FileClipboard {
                    paths: vec![source.clone()],
                    intent: FileClipboardIntent::Move,
                }),
                destination: destination.clone(),
            }),
            intent: TransferIntent::Copy,
            metadata: TransferMetadataPolicy::RequireModifiedTime,
        })
        .await;
    let done = harness.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::Failed);
    assert_eq!(done.files_completed, 0);
    assert!(source.exists());
    assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
    harness.close().await;
}
#[tokio::test]
async fn required_modified_time_copy_and_move_share_queue_and_preserve_exact_time() {
    let intents: &[TransferIntent] = if cfg!(windows) {
        &[TransferIntent::Copy, TransferIntent::RecoverableMove]
    } else {
        &[TransferIntent::Copy]
    };
    for &intent in intents {
        let (_temp, parent) = protected_fixture();
        let source = parent.join("source 文档");
        let destination = parent.join("destination");
        fs::create_dir(&destination).unwrap();
        fs::write(&source, b"all required fields and data verified").unwrap();
        File::options()
            .write(true)
            .open(&source)
            .unwrap()
            .set_times(FileTimes::new().set_modified(expected()))
            .unwrap();
        let mut harness = Harness::native();
        harness
            .controller
            .enqueue_metadata(MetadataTransferRequest {
                input: MetadataTransferInput::Paths(request(
                    1,
                    std::slice::from_ref(&source),
                    &destination,
                )),
                intent,
                metadata: TransferMetadataPolicy::RequireModifiedTime,
            })
            .await;
        let done = harness.terminal(1).await;
        assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
        assert_eq!(
            fs::metadata(destination.join("source 文档"))
                .unwrap()
                .modified()
                .unwrap(),
            expected()
        );
        assert_eq!(
            done.source_names_captured,
            u64::from(intent == TransferIntent::RecoverableMove)
        );
        if intent == TransferIntent::RecoverableMove {
            assert_eq!(done.source_files_removed, 1);
            assert!(done.source_recoveries.is_empty());
            assert!(!source.exists());
        } else {
            assert!(source.exists());
        }
        harness.close().await;
    }
}
#[tokio::test]
async fn required_modified_time_mixed_batch_is_refused_before_any_publication_or_capture() {
    let (_temp, parent) = protected_fixture();
    let source = parent.join("source");
    let folder = parent.join("folder");
    let destination = parent.join("destination");
    fs::write(&source, b"original").unwrap();
    fs::create_dir(&folder).unwrap();
    fs::create_dir(&destination).unwrap();
    for intent in [TransferIntent::Copy, TransferIntent::RecoverableMove] {
        let mut harness = Harness::native();
        harness
            .controller
            .enqueue_metadata(MetadataTransferRequest {
                input: MetadataTransferInput::Paths(request(
                    1,
                    &[source.clone(), folder.clone()],
                    &destination,
                )),
                intent,
                metadata: TransferMetadataPolicy::RequireModifiedTime,
            })
            .await;
        let done = harness.terminal(1).await;
        assert!(done.failed > 0);
        assert_eq!(done.files_completed, 0);
        assert_eq!(done.source_names_captured, 0);
        assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
        assert!(source.exists());
        harness.close().await;
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    struct MetadataPin {
        inner: Box<dyn TransferModifiedTimePin>,
        changed: Arc<AtomicBool>,
    }
    impl TransferModifiedTimePin for MetadataPin {
        fn modified(&self) -> SystemTime {
            self.inner.modified()
        }
        fn current_modified(&self, cancel: &Cancellation) -> Result<SystemTime, FsError> {
            if self.changed.load(Ordering::SeqCst) {
                Ok(SystemTime::UNIX_EPOCH)
            } else {
                self.inner.current_modified(cancel)
            }
        }
        fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
            if self.changed.load(Ordering::SeqCst) {
                Err(FsError::from_io(
                    "metadata test",
                    Path::new("source"),
                    std::io::Error::other("injected source metadata edit"),
                ))
            } else {
                self.inner.revalidate(cancel)
            }
        }
    }
    struct MetadataSource {
        inner: Box<dyn TransferSource>,
        changed: Arc<AtomicBool>,
    }
    impl TransferSource for MetadataSource {
        fn prepare_modified_time(
            &self,
            cancel: &Cancellation,
        ) -> Result<Box<dyn TransferModifiedTimePin>, FsError> {
            Ok(Box::new(MetadataPin {
                inner: self.inner.prepare_modified_time(cancel)?,
                changed: self.changed.clone(),
            }))
        }
        fn entry(&self) -> &TransferEntry {
            self.inner.entry()
        }
        fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
            self.inner.revalidate(cancel)
        }
        fn read_chunk(&mut self, data: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
            self.inner.read_chunk(data, cancel)
        }
        fn next_child(
            &mut self,
            cancel: &Cancellation,
        ) -> Result<Option<Box<dyn TransferSource>>, FsError> {
            self.inner.next_child(cancel)
        }
        fn rewind_for_validation(
            &mut self,
            cancel: &Cancellation,
        ) -> Result<Box<dyn TransferReadGuard>, FsError> {
            self.inner.rewind_for_validation(cancel)
        }
    }
    struct MetadataService {
        native: midnight_platform::LocalFileSystem,
        changed: Arc<AtomicBool>,
    }
    impl TransferService for MetadataService {
        fn prepare_move_source(
            &self,
            path: &Path,
            cancel: &Cancellation,
        ) -> Result<PreparedTransferMove, FsError> {
            let mut prepared = self.native.prepare_move_source(path, cancel)?;
            if path.file_name() == Some(OsStr::new("second")) {
                prepared.source = Box::new(MetadataSource {
                    inner: prepared.source,
                    changed: self.changed.clone(),
                });
            }
            Ok(prepared)
        }
        fn prepare_move_destination(
            &self,
            path: &Path,
            cancel: &Cancellation,
        ) -> Result<Arc<dyn TransferDirectory>, FsError> {
            self.native.prepare_move_destination(path, cancel)
        }
        fn prepare_transfer_source(
            &self,
            path: &Path,
            cancel: &Cancellation,
        ) -> Result<Box<dyn TransferSource>, FsError> {
            self.native.prepare_transfer_source(path, cancel)
        }
        fn prepare_transfer_destination(
            &self,
            path: &Path,
            cancel: &Cancellation,
        ) -> Result<Arc<dyn TransferDirectory>, FsError> {
            self.native.prepare_transfer_destination(path, cancel)
        }
    }
    #[test]
    fn metadata_change_before_or_after_capture_never_loses_original_bytes() {
        for after_capture in [false, true] {
            let (_temp, parent) = protected_fixture();
            let destination = parent.join("destination");
            fs::create_dir(&destination).unwrap();
            let paths = [parent.join("first"), parent.join("second")];
            for path in &paths {
                fs::write(path, b"original bytes").unwrap();
            }
            let request = request(1, &paths, &destination);
            let status = Arc::new(Mutex::new(initial(&CopyInput::move_paths(request.clone()))));
            let changed = Arc::new(AtomicBool::new(false));
            let service = MetadataService {
                native: midnight_platform::LocalFileSystem::new(None),
                changed: changed.clone(),
            };
            let mut job = worker::Job::prepare_move_with_metadata(
                request,
                &service,
                Cancellation::default(),
                status.clone(),
                TransferMetadataPolicy::RequireModifiedTime,
            )
            .unwrap();
            for _ in 0..300 {
                let progress = snapshot(&status);
                if (!after_capture && progress.files_completed == 1)
                    || (after_capture && progress.source_names_captured == 2)
                {
                    changed.store(true, Ordering::SeqCst);
                }
                if matches!(job.step(), worker::Step::Done) {
                    break;
                }
            }
            let done = job.finish();
            assert!(done.failed > 0, "{done:?}");
            assert_eq!(
                done.source_names_captured,
                if after_capture { 2 } else { 1 }
            );
            assert_eq!(done.source_files_removed, 1);
            if after_capture {
                for recovery in &done.source_recoveries {
                    assert_eq!(
                        fs::read(&recovery.recovery.path).unwrap(),
                        b"original bytes"
                    );
                }
            } else {
                assert!(!paths[0].exists());
                assert_eq!(fs::read(&paths[1]).unwrap(), b"original bytes");
            }
        }
    }
}
