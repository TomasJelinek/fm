//! The injected filesystem service owns native capture; errors never reopen paths.
use crate::{NativeCapture, capture};
use midnight_core::{DirectoryIndex, Entry, EntryId, EntryKind, ViewerLaunch};
use midnight_plugin_host::{CapturedFile, ContentSource, Error};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[derive(Debug)]
struct Memory;
impl ContentSource for Memory {
    fn length(&self) -> u64 {
        8
    }
    fn display_name(&self) -> String {
        "provider.txt".into()
    }
    fn version(&self) -> Vec<u8> {
        vec![1]
    }
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        let start = offset.min(8) as usize;
        let end = start.saturating_add(length as usize).min(8);
        Ok(b"provider"[start..end].to_vec())
    }
}

fn launch(root: &Path) -> ViewerLaunch {
    let entries: Arc<[Entry]> = ["first.txt", "second.txt", "hidden.txt"]
        .into_iter()
        .enumerate()
        .map(|(id, name)| Entry::new(EntryId(id as u64), root.join(name), EntryKind::File))
        .collect();
    let mut view = DirectoryIndex::from_entries(&entries);
    view.by_id.remove(&EntryId(2));
    view.by_id.insert(EntryId(0), 1);
    view.by_id.insert(EntryId(1), 0);
    ViewerLaunch {
        associations: Default::default(),
        tree: None,
        generated: None,
        preferences: Arc::new(Default::default()),
        archive: None,
        focused: entries[0].path.clone(),
        entries,
        view: Arc::new(view),
        basic: true,
        disabled_plugins: Arc::from([]),
    }
}

#[tokio::test]
async fn injected_capture_owns_focused_and_next_visible_content_without_native_files() {
    let root = tempfile::tempdir().unwrap();
    let launch = launch(root.path());
    let calls = Arc::new(Mutex::new(Vec::<PathBuf>::new()));
    let record = calls.clone();
    let source: NativeCapture = Arc::new(move |path, closed| {
        assert!(!closed.load(Ordering::Acquire));
        record.lock().unwrap().push(path.into());
        CapturedFile::from_source(Arc::new(Memory)).map_err(|error| error.to_string())
    });
    let closed = Arc::new(AtomicBool::new(false));
    let order = capture::prepare_with(&launch, &closed, source).unwrap();
    assert_eq!(order.current, 1);
    assert_eq!(order.files.len(), 2);
    let focused = order.files[order.current]
        .1
        .as_ref()
        .unwrap()
        .as_ref()
        .unwrap();
    assert_eq!(focused.read_at(0, 8).unwrap(), b"provider");
    let (next, file) = order.ready.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(next, 0);
    assert_eq!(file.unwrap().read_at(0, 8).unwrap(), b"provider");
    assert_eq!(
        *calls.lock().unwrap(),
        [
            root.path().join("first.txt"),
            root.path().join("second.txt")
        ]
    );
}

#[tokio::test]
async fn unavailable_injected_provider_never_falls_back_to_a_readable_native_file() {
    let root = tempfile::tempdir().unwrap();
    let launch = launch(root.path());
    std::fs::write(&launch.focused, b"native bytes must not be opened").unwrap();
    let source: NativeCapture = Arc::new(|_, _| Err("Provider removed".into()));
    let closed = Arc::new(AtomicBool::new(false));
    let order = capture::prepare_with(&launch, &closed, source).unwrap();
    assert_eq!(
        order.files[order.current]
            .1
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap_err(),
        "Provider removed"
    );
    assert_eq!(
        order
            .ready
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .1
            .unwrap_err(),
        "Provider removed"
    );
}

#[tokio::test]
async fn close_during_focused_capture_discards_result_and_prevents_prefetch() {
    let root = tempfile::tempdir().unwrap();
    let launch = launch(root.path());
    let calls = Arc::new(Mutex::new(0));
    let count = calls.clone();
    let source: NativeCapture = Arc::new(move |_, closed| {
        *count.lock().unwrap() += 1;
        closed.store(true, Ordering::Release);
        CapturedFile::from_source(Arc::new(Memory)).map_err(|error| error.to_string())
    });
    let closed = Arc::new(AtomicBool::new(false));
    assert!(capture::prepare_with(&launch, &closed, source).is_err());
    assert_eq!(*calls.lock().unwrap(), 1);
}

#[tokio::test]
async fn a_virtual_entry_cannot_enter_the_native_capture_route() {
    let root = tempfile::tempdir().unwrap();
    let mut launch = launch(root.path());
    let mut entries = launch.entries.to_vec();
    entries[0].virtual_node = Some(midnight_core::ArchiveNode {
        modified: None,
        id: midnight_core::VirtualEntryId { mount: 1, entry: 2 },
        parent: 1,
        raw_name: Arc::from(b"first.txt".as_slice()),
        display_name: "first.txt".into(),
        kind: EntryKind::File,
        size: Some(8),
        readable: true,
    });
    launch.entries = entries.into();
    let source: NativeCapture = Arc::new(|_, _| panic!("native capture must not run"));
    assert!(
        capture::prepare_with(&launch, &Arc::new(AtomicBool::new(false)), source)
            .err()
            .unwrap()
            .contains("non-native")
    );
}

#[test]
fn viewer_service_dispatches_injected_capture_on_a_backend_thread() {
    let root = tempfile::tempdir().unwrap();
    let launch = launch(root.path());
    let focused = launch.focused.clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    let source: NativeCapture = Arc::new(move |path, _| {
        sender
            .send((path.to_path_buf(), std::thread::current().id()))
            .unwrap();
        Err("Injected provider unavailable".into())
    });
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let service = crate::Service::with_native_capture(runtime.handle().clone(), source);
    *service.runtime.lock().unwrap() = Some(Arc::new(
        midnight_plugin_host::Runtime::discover(root.path()).unwrap(),
    ));
    *service.images.lock().unwrap() = Some(Arc::from([]));
    *service.web.lock().unwrap() = Some(Arc::from([]));
    let session = service
        .open(
            launch,
            midnight_core::ViewerAppearance {
                family: "monospace".into(),
                font_size: 14.0,
                foreground: [255; 4],
                background: [0, 0, 0, 255],
                selection: [80, 100, 180, 255],
                accent: [150, 180, 240, 255],
            },
            midnight_core::ViewerGeometry {
                width: 320,
                height: 200,
                scale: 1.0,
            },
        )
        .unwrap();
    let (path, thread) = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(path, focused);
    assert_ne!(thread, std::thread::current().id());
    session.close();
    runtime.shutdown_timeout(Duration::from_secs(2));
}

fn tree_launch(root: &Path) -> ViewerLaunch {
    use midnight_core::*;
    let mut launch = launch(root);
    launch.entries = launch
        .entries
        .iter()
        .map(|e| {
            Entry::new(
                EntryId(e.id.0 + 10),
                root.join(format!("žluťoučký-{}", e.id.0)),
                e.kind,
            )
        })
        .collect();
    launch.focused = launch.entries[0].path.clone();
    launch.view = Arc::new(DirectoryIndex::from_entries(&launch.entries));
    let snapshot = TreeRootSnapshot {
        pane: PaneId::Left,
        navigation: NavigationId(1),
        entries: launch.entries.clone(),
        index: launch.view.clone(),
    };
    let rows = [1usize, 0]
        .into_iter()
        .map(|position| {
            let mut entry = launch.entries[position].clone();
            // Occurrence IDs collide with the other root file's original ID.
            entry.id = EntryId((1 - position) as u64 + 10);
            TreeDisplayRow {
                id: TreeRowId(entry.id.0),
                entry,
                parent: None,
                depth: 0,
                disclosure: TreeDisclosure::None,
                expanded: false,
                selectable: true,
                navigable: false,
                context_only: false,
                error: None,
                error_details: None,
                original_root_entry: Some(launch.entries[position].id),
                prefix_end: 0,
                search_match: false,
            }
        })
        .collect();
    let view = Arc::new(
        TreeProjection::new(
            snapshot,
            TreeViewStamp {
                pane: PaneId::Left,
                navigation: NavigationId(1),
                session: TreeSessionId(1),
                revision: TreeRevision(1),
            },
            TreeLayoutRevision(1),
            rows,
        )
        .unwrap(),
    );
    launch.tree = Some(TreeViewer {
        view,
        focused: TreeRowId(11),
    });
    launch
}

#[tokio::test]
async fn tree_capture_uses_visible_alias_order_and_injected_native_provider() {
    let root = tempfile::tempdir().unwrap();
    let launch = tree_launch(root.path());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let record = calls.clone();
    let source: NativeCapture = Arc::new(move |path, _| {
        record.lock().unwrap().push(path.to_path_buf());
        CapturedFile::from_source(Arc::new(Memory)).map_err(|e| e.to_string())
    });
    let order = capture::prepare_with(&launch, &Arc::new(AtomicBool::new(false)), source).unwrap();
    assert_eq!(
        order.files.iter().map(|f| &f.0).collect::<Vec<_>>(),
        [&launch.entries[1].path, &launch.entries[0].path]
    );
    assert_eq!(order.current, 1);
    assert_eq!(
        order.ready.recv_timeout(Duration::from_secs(2)).unwrap().0,
        0
    );
    assert_eq!(
        *calls.lock().unwrap(),
        [
            launch.entries[0].path.clone(),
            launch.entries[1].path.clone()
        ]
    );
}

#[tokio::test]
async fn tree_capture_rejects_foreign_roots_and_malformed_focus_before_native_capture() {
    let root = tempfile::tempdir().unwrap();
    let original = tree_launch(root.path());
    let reject: NativeCapture = Arc::new(|_, _| panic!("invalid tree must not capture"));
    for case in 0..9 {
        let mut launch = original.clone();
        if case == 0 {
            launch.entries = launch.entries.to_vec().into();
        } else if case == 1 {
            launch.view = Arc::new((*launch.view).clone());
        } else if case == 2 {
            launch.focused = root.path().join("foreign");
        } else {
            let tree = launch.tree.as_mut().unwrap();
            let view = Arc::make_mut(&mut tree.view);
            let row = &mut Arc::make_mut(&mut view.rows)[1];
            match case {
                3 => row.depth = 1,
                4 => row.parent = Some(midnight_core::TreeRowId(10)),
                5 => row.selectable = false,
                6 => row.context_only = true,
                7 => row.original_root_entry = None,
                _ => row.entry.path = root.path().join("substituted"),
            }
        }
        assert!(
            capture::prepare_with(&launch, &Arc::new(AtomicBool::new(false)), reject.clone())
                .is_err(),
            "case {case}"
        );
    }
}

#[test]
fn tree_viewer_accepts_current_root_actions_and_rejects_the_obsolete_seed_snapshot() {
    use midnight_core::*;
    let directory = tempfile::tempdir().unwrap();
    let mut launch = tree_launch(directory.path());
    let old = launch.tree.as_ref().unwrap().view.clone();
    let added = Entry::new(
        EntryId(90),
        directory.path().join("new-žluťoučký.txt"),
        EntryKind::File,
    );
    let mut entries = old.root.entries.to_vec();
    entries.push(added.clone());
    let actions = TreeRootSnapshot {
        entries: entries.clone().into(),
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        ..old.root.clone()
    };
    let mut rows = old.rows.to_vec();
    let mut row = rows[0].clone();
    row.id = TreeRowId(77);
    row.entry = added.clone();
    row.entry.id = EntryId(77);
    row.original_root_entry = Some(added.id);
    rows.push(row);
    let current = Arc::new(
        TreeProjection::new_with_root_actions(
            old.root.clone(),
            actions.clone(),
            TreeViewStamp {
                revision: TreeRevision(2),
                ..old.stamp
            },
            TreeLayoutRevision(2),
            rows,
        )
        .unwrap(),
    );
    launch.tree = Some(TreeViewer {
        view: current,
        focused: TreeRowId(77),
    });
    launch.entries = actions.entries;
    launch.view = actions.index;
    launch.focused = added.path.clone();
    let closed = AtomicBool::new(false);
    let normalized = crate::capture_tree::normalize(&launch, &closed).unwrap();
    assert_eq!(normalized.entries.len(), 3);
    assert_eq!(normalized.entries.last().unwrap().path, added.path);
    assert_eq!(normalized.focused, added.path);
    launch.entries = old.root.entries.clone();
    launch.view = old.root.index.clone();
    assert!(crate::capture_tree::normalize(&launch, &closed).is_err());
}
