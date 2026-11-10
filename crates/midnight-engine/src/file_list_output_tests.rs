use super::*;
use std::{fs, path::PathBuf};

fn fixture() -> tempfile::TempDir {
    // Native publication capabilities must be tested on the runner's selected
    // filesystem, not necessarily the checkout mount (e.g. Windows DrvFs in WSL).
    let root = std::env::temp_dir().join("midnight-file-list/_build/tmp");
    fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix("file-list-output-")
        .tempdir_in(root.canonicalize().unwrap())
        .unwrap()
}

fn source(
    provider: &dyn TransferService,
    root: &Path,
    cancel: &Cancellation,
) -> Box<dyn TransferSource> {
    provider
        .prepare_transfer_source(&root.join("generated.txt"), cancel)
        .unwrap()
}

fn saved(outcome: FileListSaveOutcome) -> FileListSaveReceipt {
    match outcome {
        FileListSaveOutcome::Saved(receipt) => receipt,
        FileListSaveOutcome::Conflict(_) => panic!("unexpected conflict"),
    }
}

fn conflict(outcome: FileListSaveOutcome) -> FileListDestinationPin {
    match outcome {
        FileListSaveOutcome::Conflict(pin) => pin,
        FileListSaveOutcome::Saved(_) => panic!("must wait for explicit conflict consent"),
    }
}

#[test]
fn creates_verified_unicode_list_without_replacing_an_existing_name() {
    let root = fixture();
    let provider = midnight_platform::LocalFileSystem::new(None);
    let cancel = Cancellation::default();
    let bytes = "český soubor\r\n文档\r\n".as_bytes();
    fs::write(root.path().join("generated.txt"), bytes).unwrap();
    let destination = root.path().join("seznam 文档.txt");
    let receipt = saved(
        save_file_list(
            source(&provider, root.path(), &cancel),
            &destination,
            false,
            None,
            &provider,
            &cancel,
        )
        .unwrap(),
    );
    assert_eq!(fs::read(&destination).unwrap(), bytes);
    assert_eq!(receipt.path, destination);
    assert_eq!(receipt.verified_bytes, bytes.len() as u64);
    assert!(receipt.recovery.is_empty());
    assert!(receipt.warning.is_none());
    let pin = conflict(
        save_file_list(
            source(&provider, root.path(), &cancel),
            &destination,
            false,
            None,
            &provider,
            &cancel,
        )
        .unwrap(),
    );
    assert_eq!(pin.entry().item.metadata.size, Some(bytes.len() as u64));
    assert_eq!(fs::read(&destination).unwrap(), bytes);
}

#[test]
fn confirmed_replace_and_append_preserve_the_original_until_verified_publication() {
    for append in [false, true] {
        let root = fixture();
        let provider = midnight_platform::LocalFileSystem::new(None);
        let cancel = Cancellation::default();
        fs::write(root.path().join("generated.txt"), "文档\n").unwrap();
        let destination = root.path().join("list.txt");
        fs::write(&destination, b"old\n").unwrap();
        let pin = conflict(
            save_file_list(
                source(&provider, root.path(), &cancel),
                &destination,
                append,
                None,
                &provider,
                &cancel,
            )
            .unwrap(),
        );
        assert_eq!(fs::read(&destination).unwrap(), b"old\n");
        let receipt = saved(
            save_file_list(
                source(&provider, root.path(), &cancel),
                &destination,
                append,
                Some(pin),
                &provider,
                &cancel,
            )
            .unwrap_or_else(|error| panic!("append={append}: {error:?}")),
        );
        let expected = if append { "old\n文档\n" } else { "文档\n" };
        assert_eq!(fs::read(&destination).unwrap(), expected.as_bytes());
        assert_eq!(receipt.verified_bytes, expected.len() as u64);
        assert_eq!(receipt.recovery.len(), 1);
        assert_eq!(fs::read(&receipt.recovery[0]).unwrap(), b"old\n");
        assert!(receipt.warning.is_none());
    }
}

#[test]
fn changed_or_disappeared_conflict_never_reuses_old_confirmation() {
    for disappear in [false, true] {
        let root = fixture();
        let provider = midnight_platform::LocalFileSystem::new(None);
        let cancel = Cancellation::default();
        fs::write(root.path().join("generated.txt"), b"new\n").unwrap();
        let destination = root.path().join("list.txt");
        fs::write(&destination, b"original\n").unwrap();
        let pin = conflict(
            save_file_list(
                source(&provider, root.path(), &cancel),
                &destination,
                true,
                None,
                &provider,
                &cancel,
            )
            .unwrap(),
        );
        if disappear {
            fs::remove_file(&destination).unwrap();
        } else {
            fs::write(&destination, b"changed externally\n").unwrap();
        }
        assert!(
            save_file_list(
                source(&provider, root.path(), &cancel),
                &destination,
                true,
                Some(pin),
                &provider,
                &cancel,
            )
            .is_err()
        );
        if disappear {
            assert!(!destination.exists());
        } else {
            assert_eq!(fs::read(&destination).unwrap(), b"changed externally\n");
        }
    }
}

#[test]
fn failed_or_cancelled_append_keeps_the_original_bytes() {
    for cancel_after_read in [false, true] {
        let root = fixture();
        let provider = midnight_platform::LocalFileSystem::new(None);
        let cancel = Cancellation::default();
        fs::write(root.path().join("generated.txt"), b"generated\n").unwrap();
        let destination = root.path().join("list.txt");
        fs::write(&destination, b"original\n").unwrap();
        let pin = conflict(
            save_file_list(
                source(&provider, root.path(), &cancel),
                &destination,
                true,
                None,
                &provider,
                &cancel,
            )
            .unwrap(),
        );
        let failing = FailingSource {
            inner: source(&provider, root.path(), &cancel),
            read: false,
            cancel_after_read,
        };
        assert!(
            save_file_list(
                Box::new(failing),
                &destination,
                true,
                Some(pin),
                &provider,
                &cancel,
            )
            .is_err()
        );
        assert_eq!(fs::read(&destination).unwrap(), b"original\n");
    }
}

#[test]
fn rejects_relative_parent_traversal_directory_and_source_alias_targets() {
    let root = fixture();
    let provider = midnight_platform::LocalFileSystem::new(None);
    let cancel = Cancellation::default();
    fs::write(root.path().join("generated.txt"), b"generated\n").unwrap();
    fs::create_dir(root.path().join("folder")).unwrap();
    // PathBuf::join normalizes parent steps in verbatim Windows paths. Preserve
    // the actual input here so the validation test exercises an unnormalized path.
    let mut traversal = root.path().join("folder").into_os_string();
    traversal.push(std::path::MAIN_SEPARATOR_STR);
    traversal.push("..");
    traversal.push(std::path::MAIN_SEPARATOR_STR);
    traversal.push("escaped.txt");
    for destination in [
        PathBuf::from("relative.txt"),
        PathBuf::from(traversal),
        root.path().join("folder"),
        root.path().join("generated.txt"),
    ] {
        assert!(
            save_file_list(
                source(&provider, root.path(), &cancel),
                &destination,
                false,
                None,
                &provider,
                &cancel,
            )
            .is_err(),
            "{}",
            destination.display()
        );
    }
    assert_eq!(
        fs::read(root.path().join("generated.txt")).unwrap(),
        b"generated\n"
    );
}

#[test]
fn confirmation_cannot_change_output_path_or_append_mode() {
    for change_path in [false, true] {
        let root = fixture();
        let provider = midnight_platform::LocalFileSystem::new(None);
        let cancel = Cancellation::default();
        fs::write(root.path().join("generated.txt"), b"new\n").unwrap();
        let destination = root.path().join("list.txt");
        fs::write(&destination, b"original\n").unwrap();
        let pin = conflict(
            save_file_list(
                source(&provider, root.path(), &cancel),
                &destination,
                false,
                None,
                &provider,
                &cancel,
            )
            .unwrap(),
        );
        let other = root.path().join("other.txt");
        let Err(error) = save_file_list(
            source(&provider, root.path(), &cancel),
            if change_path { &other } else { &destination },
            !change_path,
            Some(pin),
            &provider,
            &cancel,
        ) else {
            panic!("changed confirmation must fail")
        };
        assert_eq!(error.kind, io::ErrorKind::InvalidInput);
        assert!(error.message.contains("confirmed output"));
        assert_eq!(fs::read(&destination).unwrap(), b"original\n");
        assert!(!other.exists());
    }
}

#[test]
fn publication_uncertainty_reports_recoveries_and_never_claims_an_unpublished_list() {
    let destination = PathBuf::from("output.txt");
    let recovery = PathBuf::from("retained-original.txt");
    for published in [None, Some(destination.clone())] {
        let result = receipt(
            TransferPublication::RecoveryRequired {
                published: published.clone(),
                recovery: vec![TransferRecovery {
                    path: recovery.clone(),
                    identity: None,
                }],
                error: FsError::from_io(
                    "publish",
                    &destination,
                    io::Error::other("partial publication"),
                ),
            },
            9,
        );
        if published.is_some() {
            let output = result.unwrap();
            assert_eq!(output.path, destination);
            assert_eq!(output.recovery.as_slice(), std::slice::from_ref(&recovery));
            assert!(
                output
                    .warning
                    .unwrap()
                    .message
                    .contains("partial publication")
            );
        } else {
            let Err(error) = result else {
                panic!("unpublished output cannot be Saved")
            };
            assert!(error.message.contains("partial publication"));
            assert!(error.message.contains("retained-original.txt"));
        }
    }
}

struct FailingSource {
    inner: Box<dyn TransferSource>,
    read: bool,
    cancel_after_read: bool,
}

impl TransferSource for FailingSource {
    fn entry(&self) -> &TransferEntry {
        self.inner.entry()
    }

    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.revalidate(cancel)
    }

    fn read_chunk(&mut self, buffer: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        if self.read {
            return Err(FsError::from_io(
                "read file list",
                &self.entry().item.path,
                io::Error::other("injected source failure"),
            ));
        }
        self.read = true;
        let count = self.inner.read_chunk(buffer, cancel)?;
        if self.cancel_after_read {
            cancel.cancel();
        }
        Ok(count)
    }

    fn next_child(&mut self, _: &Cancellation) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        unreachable!("generated output is a regular file")
    }
}
