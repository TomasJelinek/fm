//! One Copy or Move takes at most the native roots it can hold open, however long the
//! file list that the clipboard, a drop or a panel selection hands it.
use super::*;

fn paths(count: usize) -> CopyRequest {
    let root = std::env::current_dir().unwrap();
    CopyRequest {
        start_policy: TransferStartPolicy::Immediate,
        verification: TransferVerification::FullSha256,
        id: OperationId(1),
        clipboard: FileClipboard {
            paths: (0..count)
                .map(|index| root.join(format!("source-{index}")))
                .collect(),
            intent: FileClipboardIntent::Copy,
        },
        destination: root.join("destination"),
    }
}

fn refused(error: &FsError, count: usize) {
    assert_eq!(error.kind, io::ErrorKind::InvalidInput);
    assert!(
        error.message.contains(&format!(
            "Copy and Move take at most {MAX_NATIVE_TRANSFER_ROOTS} items at once"
        )) && error.message.contains(&format!("these are {count}")),
        "{error}"
    );
}

#[test]
fn a_file_list_of_more_native_roots_than_one_copy_holds_open_is_refused_at_admission() {
    // A file list itself takes any number of paths within its bytes (the clipboard
    // refused more than 4,096); Copy bounds the roots it prepares and keeps open.
    CopyInput::from(paths(MAX_NATIVE_TRANSFER_ROOTS))
        .validate()
        .unwrap();
    let count = MAX_NATIVE_TRANSFER_ROOTS + 1;
    refused(
        &CopyInput::from(paths(count)).validate().unwrap_err(),
        count,
    );
}

#[test]
fn a_panel_selection_of_more_native_roots_is_refused_before_any_root_is_prepared() {
    let count = MAX_NATIVE_TRANSFER_ROOTS + 1;
    let root = std::env::current_dir().unwrap();
    let entries: Vec<_> = (0..count)
        .map(|index| {
            Entry::new(
                EntryId(index as u64),
                root.join(format!("source-{index}")),
                EntryKind::File,
            )
        })
        .collect();
    let view = Arc::new(DirectoryIndex::from_entries(&entries));
    let mut bits = vec![u64::MAX; count.div_ceil(64)];
    if !count.is_multiple_of(64) {
        *bits.last_mut().unwrap() = (1 << (count % 64)) - 1;
    }
    let selection = Arc::new(SelectionSnapshot::from_bits(view.clone(), bits).unwrap());
    let input = CopyInput::from(SelectedCopyRequest {
        start_policy: TransferStartPolicy::Immediate,
        verification: TransferVerification::FullSha256,
        id: OperationId(1),
        files: SelectedFiles {
            view,
            selection,
            fallback: None,
        },
        destination: root.join("destination"),
    });
    // The service is never asked to prepare a root.
    let service = midnight_platform::LocalFileSystem::new(None);
    match input.resolve(&service, &Cancellation::default()) {
        Err(error) => refused(&error, count),
        Ok(_) => panic!("a selection beyond the native root bound was resolved"),
    }
}
