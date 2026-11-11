use super::*;
use crate::providers::captured_archive_root;
use midnight_core::{
    ArchiveAddress, ArchiveLocation, ArchiveMount, ArchiveNode, FsError, VirtualEntryId,
};
use std::{collections::HashMap, io, path::Path, sync::atomic::AtomicBool};

#[derive(Debug)]
struct ArchiveFixture {
    rows: HashMap<u64, Vec<ArchiveNode>>,
    invalid: AtomicBool,
}

impl ArchiveMount for ArchiveFixture {
    fn id(&self) -> u64 {
        73
    }

    fn list(&self, directory: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        self.validate()?;
        Ok(self.rows.get(&directory).cloned().unwrap_or_default())
    }

    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        Err(FsError::from_io(
            "archive fixture",
            Path::new(""),
            io::ErrorKind::Unsupported.into(),
        ))
    }

    fn validate(&self) -> Result<(), FsError> {
        if self.invalid.load(Ordering::Relaxed) {
            Err(FsError::from_io(
                "archive fixture",
                Path::new(""),
                io::Error::other("source changed"),
            ))
        } else {
            Ok(())
        }
    }
}

fn archive_node(entry: u64, parent: u64, raw_name: &[u8]) -> ArchiveNode {
    ArchiveNode {
        modified: None,
        id: VirtualEntryId { mount: 73, entry },
        parent,
        raw_name: Arc::from(raw_name),
        display_name: "same caption".into(),
        kind: EntryKind::Directory,
        size: None,
        readable: false,
    }
}

fn archive_fixture() -> (Session, Arc<ArchiveFixture>, tempfile::TempDir) {
    archive_fixture_names(b"parent-\xff", b"child-\xfe")
}
fn archive_fixture_names(
    parent_name: &[u8],
    child_name: &[u8],
) -> (Session, Arc<ArchiveFixture>, tempfile::TempDir) {
    let directory = tempfile::tempdir().unwrap();
    let parent = archive_node(4, 1, parent_name);
    let child = archive_node(7, 4, child_name);
    let mount = Arc::new(ArchiveFixture {
        rows: HashMap::from([(1, vec![parent.clone()]), (4, vec![child])]),
        invalid: AtomicBool::new(false),
    });
    let location = ArchiveLocation {
        mount: Some(mount.clone()),
        address: ArchiveAddress {
            provider: "test.archive".into(),
            format: None,
            container: directory.path().join("container.zip"),
            source_version: vec![3, 8],
            components: Vec::new(),
        },
        directory: 1,
        trail: vec![(1, "container.zip".into())],
    };
    let entries: Arc<[Entry]> = Arc::from([Entry::from_archive(parent)]);
    let root = TreeRootSnapshot {
        pane: PaneId::Right,
        navigation: NavigationId(29),
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
    };
    let opened = captured_archive_root(&location, &root.entries, &Cancellation::default()).unwrap();
    let session = Session::new(
        root,
        opened,
        Arc::new(ProviderRegistry::default()),
        WorkerBudget::new(Arc::new(Semaphore::new(2))),
        ProjectionOptions::default(),
        &Cancellation::default(),
    )
    .unwrap();
    (session, mount, directory)
}

#[test]
fn expanded_archive_descendant_keeps_raw_ancestry_and_live_mount_after_navigation() {
    let (mut session, mount, _directory) = archive_fixture();
    let parent = session.projection().rows[0].id;
    let expanded = expand(&mut session, parent);
    let child = expanded.view.rows[1].id;
    assert_eq!(
        expanded.view.rows[0].entry.display_name,
        expanded.view.rows[1].entry.display_name
    );
    let selected = select(&mut session, TreeSelectionAction::Toggle(child));
    let batch = capture(&session, &selected, None, &Cancellation::default()).unwrap();
    assert_eq!(batch.targets[0].row, child);
    drop(session);
    let ProviderTarget::Archive(target) = batch.targets[0].captured.target.value().unwrap() else {
        panic!("typed archive target expected")
    };
    assert_eq!(target.location.directory, 7);
    assert_eq!(
        target.location.address.components,
        [b"parent-\xff".to_vec(), b"child-\xfe".to_vec()]
    );
    assert_eq!(
        target
            .location
            .trail
            .iter()
            .map(|item| item.0)
            .collect::<Vec<_>>(),
        [1, 4, 7]
    );
    assert!(target.member.is_none());
    assert_eq!(target.original.as_ref().unwrap().id.entry, 7);
    assert_eq!(
        target.original.as_ref().unwrap().raw_name.as_ref(),
        b"child-\xfe"
    );
    let retained_mount = target.location.mount.as_ref().unwrap().clone();
    assert!(retained_mount.validate().is_ok());
    // Retaining context does not make changed source contents valid for an operation.
    mount.invalid.store(true, Ordering::Relaxed);
    assert!(retained_mount.validate().is_err());
    mount.invalid.store(false, Ordering::Relaxed);
    drop(batch);
    assert!(retained_mount.validate().is_err());
}

#[test]
fn archive_clipboard_capture_never_exports_a_container_or_temporary_path() {
    for intent in [
        midnight_core::FileClipboardIntent::Copy,
        midnight_core::FileClipboardIntent::Move,
    ] {
        let (mut session, _mount, _directory) = archive_fixture();
        let parent = session.projection().rows[0].id;
        let expanded = expand(&mut session, parent);
        let capacity = Arc::new(Semaphore::new(4));
        let request = midnight_core::TreeClipboardRequest {
            scope: midnight_core::TreeClipboardScope::SelectionOrFallback,
            intent,
            request: TreeRequestId(session.last_request.0 + 1),
            selection_revision: expanded.selection_revision,
            selection: expanded.selection,
            fallback: Some(expanded.view.rows[1].id),
            view: expanded.view,
            cancel: Cancellation::default(),
        };
        assert!(matches!(
            session.capture_clipboard(&request, &capacity, &Cancellation::default()),
            Err(ProviderError::Unsupported)
        ));
        assert_eq!(capacity.available_permits(), 4);
    }
}

#[test]
fn checksum_capture_rejects_selected_archive_descendant_without_native_fallback() {
    let (mut session, _mount, _directory) = archive_fixture();
    let parent = session.projection().rows[0].id;
    let expanded = expand(&mut session, parent);
    let selected = select(
        &mut session,
        TreeSelectionAction::Toggle(expanded.view.rows[1].id),
    );
    let capacity = Arc::new(Semaphore::new(1));
    let request = midnight_core::TreeChecksumRequest {
        request: TreeRequestId(session.last_request.0 + 1),
        selection_revision: selected.selection_revision,
        selection: selected.selection,
        fallback: None,
        view: selected.view,
        cancel: Cancellation::default(),
    };
    assert!(matches!(
        session.capture_checksums(&request, &capacity, &Cancellation::default()),
        Err(ProviderError::Unsupported)
    ));
    assert_eq!(capacity.available_permits(), 1);
}

#[test]
fn changed_archive_source_rejects_capture_without_a_native_target_fallback() {
    let (mut session, mount, _directory) = archive_fixture();
    let parent = session.projection().rows[0].id;
    let expanded = expand(&mut session, parent);
    let selected = select(
        &mut session,
        TreeSelectionAction::Toggle(expanded.view.rows[1].id),
    );
    let owners = Arc::strong_count(&session.registration);
    mount.invalid.store(true, Ordering::Relaxed);
    assert!(matches!(
        capture(&session, &selected, None, &Cancellation::default()),
        Err(ProviderError::Failed(_))
    ));
    assert_eq!(Arc::strong_count(&session.registration), owners);
}

struct NoNativeIo;
impl midnight_core::DirectoryService for NoNativeIo {
    fn read_directory(
        &self,
        _: &Path,
        _: &Cancellation,
        _: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        panic!("archive sizing cannot enumerate native directories")
    }
    fn entry_metadata(&self, _: &Path) -> Result<midnight_core::EntryMetadata, FsError> {
        panic!("archive sizing cannot inspect native metadata")
    }
}
impl midnight_core::VolumeService for NoNativeIo {
    fn list_volumes(&self) -> Result<Vec<midnight_core::Volume>, FsError> {
        panic!("no native volumes")
    }
}
impl midnight_core::StorageService for NoNativeIo {
    fn space_info(&self, _: &Path) -> Result<midnight_core::SpaceInfo, FsError> {
        panic!("no native storage query")
    }
}

async fn archive_size_result(
    captured: CapturedSelection,
    request: TreeRequestId,
    row: TreeRowId,
) -> midnight_core::FolderSize {
    let stamp = captured.view.stamp;
    let mut state =
        crate::tree_sizes::State::new(Arc::new(NoNativeIo), Arc::new(tokio::sync::Notify::new()));
    state.enqueue(request, captured, &Cancellation::default());
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            for event in state.tick(Some(stamp)) {
                if let midnight_core::TreeSizeEvent::Snapshot { sizes } = event {
                    state.acknowledge(sizes.stamp, sizes.version);
                    if let Some(size) = sizes.get(row)
                        && size.finished
                        && !sizes.running(row)
                    {
                        return size.clone();
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("archive size final publication")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn archive_size_source_revoked_after_capture_is_partial_never_successful_zero() {
    let (mut session, mount, _directory) = archive_fixture();
    let publication = session.publication().unwrap();
    session.commit(&publication.view);
    let row = publication.view.rows[0].id;
    let request = midnight_core::TreeSizeRequest {
        view: publication.view,
        request: TreeRequestId(1),
        selection_revision: publication.selection_revision,
        selection: publication.selection,
        scope: midnight_core::TreeSizeScope::Single(row),
    };
    let captured = session
        .capture_sizes(&request, &Cancellation::default())
        .unwrap();
    mount.invalid.store(true, Ordering::Relaxed);
    let size = archive_size_result(captured, request.request, row).await;
    assert!(size.finished);
    assert!(size.errors > 0);
    assert_eq!(size.bytes, 0);
    let warnings = size
        .warnings
        .as_ref()
        .expect("the provider failure is inspectable");
    let source = warnings.snapshot(&Cancellation::default()).unwrap();
    let mut text = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let count = source.read_at(text.len() as u64, &mut buffer).unwrap();
        if count == 0 {
            break;
        }
        text.extend_from_slice(&buffer[..count]);
    }
    let text = String::from_utf8(text).unwrap();
    assert!(
        text.contains("source changed"),
        "preserve the actual provider reason: {text}"
    );
    assert!(
        text.contains("parent-\\xff"),
        "preserve the raw archive occurrence: {text}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn archive_single_size_uses_exact_raw_child_even_when_parent_is_marked() {
    let (mut session, _mount, _directory) = archive_fixture();
    let parent = session.projection().rows[0].id;
    let expanded = expand(&mut session, parent);
    let child = expanded.view.rows[1].id;
    let selected = select(&mut session, TreeSelectionAction::Toggle(parent));
    let request = midnight_core::TreeSizeRequest {
        view: selected.view,
        request: TreeRequestId(2),
        selection_revision: selected.selection_revision,
        selection: selected.selection,
        scope: midnight_core::TreeSizeScope::Single(child),
    };
    let captured = session
        .capture_sizes(&request, &Cancellation::default())
        .unwrap();
    assert_eq!(captured.targets.len(), 1);
    assert_eq!(captured.targets[0].row, child);
    let size = archive_size_result(captured, request.request, child).await;
    assert!(size.finished);
    assert_eq!(size.errors, 0);
    assert_eq!(size.bytes, 0);
}

#[test]
fn copied_archive_nested_directory_keeps_original_basename_after_navigation() {
    let (mut session, _mount, _directory) = archive_fixture_names(b"parent", b"child");
    let parent = session.projection().rows[0].id;
    let expanded = expand(&mut session, parent);
    let child = expanded.view.rows[1].id;
    let selected = select(&mut session, TreeSelectionAction::Toggle(child));
    let capacity = Arc::new(Semaphore::new(4));
    let captured = session
        .capture_transfer(
            &midnight_core::TreeTransferRequest {
                intent: midnight_core::TransferIntent::Copy,
                view: selected.view,
                request: TreeRequestId(2),
                selection_revision: selected.selection_revision,
                selection: selected.selection,
                fallback: None,
                cancel: Cancellation::default(),
            },
            &capacity,
            &Cancellation::default(),
        )
        .unwrap();
    drop(session);
    assert!(!captured.metadata_available);
    let service = midnight_platform::LocalFileSystem::new(None);
    let midnight_core::PreparedCapturedTransfer::Copy(mut roots) = captured
        .sources
        .prepare(&service, &Cancellation::default())
        .unwrap()
    else {
        panic!("expected Copy")
    };
    let mut root = roots.pop_front().unwrap();
    assert_eq!(root.entry().item.path, Path::new("child"));
    assert_eq!(
        root.virtual_identity(),
        Some(VirtualEntryId {
            mount: 73,
            entry: 7
        })
    );
    assert!(root.native_identity().is_none());
    assert_eq!(root.tree_copy_occurrence().unwrap().row, child);
    assert!(root.next_child(&Cancellation::default()).unwrap().is_none());
    drop(captured);
    assert_eq!(capacity.available_permits(), 3);
    drop(root);
    assert_eq!(capacity.available_permits(), 4);
}
#[test]
fn archive_overlapping_selected_copy_roots_reject_before_native_preparation() {
    let (mut session, _mount, _directory) = archive_fixture_names(b"parent", b"child");
    let parent = session.projection().rows[0].id;
    let expanded = expand(&mut session, parent);
    let child = expanded.view.rows[1].id;
    select(&mut session, TreeSelectionAction::Toggle(parent));
    let selected = select(&mut session, TreeSelectionAction::Toggle(child));
    assert_eq!(selected.selection.len(), 2);
    let captured = session
        .capture_transfer(
            &midnight_core::TreeTransferRequest {
                intent: midnight_core::TransferIntent::Copy,
                view: selected.view,
                request: TreeRequestId(2),
                selection_revision: selected.selection_revision,
                selection: selected.selection,
                fallback: None,
                cancel: Cancellation::default(),
            },
            &Arc::new(Semaphore::new(4)),
            &Cancellation::default(),
        )
        .unwrap();
    let service = midnight_platform::LocalFileSystem::new(None);
    assert!(
        captured
            .sources
            .prepare(&service, &Cancellation::default())
            .is_err()
    );
}

#[test]
fn archive_move_capture_rejects_before_transfer_preparation() {
    let (mut session, _mount, directory) = archive_fixture_names(b"parent", b"child");
    // A native name that looks like the archive occurrence must remain untouched.
    let lookalike = directory.path().join("parent");
    std::fs::write(&lookalike, b"native winner").unwrap();
    let publication = session.publication().unwrap();
    assert!(session.commit(&publication.view));
    let result = session.capture_transfer(
        &midnight_core::TreeTransferRequest {
            intent: midnight_core::TransferIntent::Move,
            fallback: Some(publication.view.rows[0].id),
            view: publication.view,
            request: TreeRequestId(1),
            selection_revision: publication.selection_revision,
            selection: publication.selection,
            cancel: Cancellation::default(),
        },
        &Arc::new(Semaphore::new(4)),
        &Cancellation::default(),
    );
    assert!(matches!(result, Err(ProviderError::Unsupported)));
    assert_eq!(std::fs::read(lookalike).unwrap(), b"native winner");
}

#[path = "tree_editor_archive_tests.rs"]
mod editor_tests;
