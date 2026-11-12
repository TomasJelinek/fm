//! Verification choices travel through real admission and observable readback.
use super::*;
use std::collections::VecDeque;

#[derive(Debug)]
struct CapturedPath(PathBuf);
impl CapturedTransferSources for CapturedPath {
    fn intent(&self) -> TransferIntent {
        TransferIntent::Copy
    }
    fn root_count(&self) -> usize {
        1
    }
    fn metadata_available(&self) -> bool {
        true
    }
    fn prepare(
        &self,
        service: &dyn TransferService,
        cancel: &Cancellation,
    ) -> Result<PreparedCapturedTransfer, FsError> {
        Ok(PreparedCapturedTransfer::Copy(VecDeque::from([
            service.prepare_transfer_source(&self.0, cancel)?
        ])))
    }
}

#[tokio::test]
async fn queued_path_selected_and_captured_copy_preserve_verification_choice() {
    for route in 0..3 {
        for mode in [
            TransferVerification::FullSha256,
            TransferVerification::SizeAndSamples,
        ] {
            let temp = fixture();
            let source = temp.path().join("source");
            let destination = temp.path().join("destination");
            let bytes = vec![42; 400_000];
            fs::write(&source, &bytes).unwrap();
            fs::create_dir(&destination).unwrap();
            let mut harness = Harness::native();
            let reservation = harness.io.clone().acquire_owned().await.unwrap();
            match route {
                0 => {
                    let mut input = request(1, std::slice::from_ref(&source), &destination);
                    input.verification = mode;
                    harness.controller.enqueue(input).await;
                }
                1 => {
                    let entry = Entry::new(EntryId(1), source.clone(), EntryKind::File);
                    let mut input = selected_request(1, &[entry], &destination);
                    input.verification = mode;
                    harness.controller.enqueue_selected(input).await;
                }
                _ => {
                    harness
                        .controller
                        .enqueue_captured(CapturedTransferRequest {
                            verification: mode,
                            id: OperationId(1),
                            sources: Arc::new(CapturedPath(source.clone())),
                            destination: destination.clone(),
                            metadata: TransferMetadataPolicy::ContentOnly,
                        })
                        .await
                }
            }
            drop(reservation);
            let done = harness.terminal(1).await;
            harness.close().await;
            assert_eq!(
                done.state,
                CopyOperationState::Completed,
                "route={route}, mode={mode:?}: {done:?}"
            );
            assert_eq!(done.copied_bytes, bytes.len() as u64);
            assert_eq!(done.verification, mode);
            assert_eq!(
                done.verified_bytes,
                if mode == TransferVerification::FullSha256 {
                    bytes.len() as u64
                } else {
                    3 * TRANSFER_SAMPLE_BYTES as u64
                }
            );
            assert_eq!(fs::read(destination.join("source")).unwrap(), bytes);
        }
    }
}

#[tokio::test]
async fn shared_clipboard_cross_volume_move_uses_captured_sample_policy_before_removal() {
    let (_temp, parent) = moves::protected_fixture();
    let source = parent.join("source");
    let destination = parent.join("destination");
    fs::write(&source, vec![7; 400_000]).unwrap();
    fs::create_dir(&destination).unwrap();
    let mut harness = Harness::new(Some(Arc::new(moves::CrossDeviceService(
        midnight_platform::LocalFileSystem::new(None),
    ))));
    harness
        .controller
        .enqueue_clipboard_move(ClipboardMoveRequest {
            verification: TransferVerification::SizeAndSamples,
            id: OperationId(1),
            clipboard: Arc::new(FileClipboard {
                paths: vec![source.clone()],
                intent: FileClipboardIntent::Move,
            }),
            destination: destination.clone(),
        })
        .await;
    let done = harness.terminal(1).await;
    harness.close().await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.verification, TransferVerification::SizeAndSamples);
    assert_eq!(done.verified_bytes, 3 * TRANSFER_SAMPLE_BYTES as u64);
    assert!(done.validation_bytes > 0 && done.validation_bytes < 4 * 400_000);
    assert_eq!(done.source_files_removed, 1);
    assert_eq!(done.native_entries_moved, 0);
    assert!(!source.exists());
    assert_eq!(
        fs::metadata(destination.join("source")).unwrap().len(),
        400_000
    );
}
