#![cfg(target_os = "linux")]
#![forbid(unsafe_code)]
//! Real symlinks across the native service, captured root and production tree controller.
use midnight_core::{
    Cancellation, Command, DirectoryService, EntryKind, Event, FileClipboardIntent, NavigationId,
    PaneId, SelectionRevision, SortSpec, TreeBranchAction, TreeClipboardRequest,
    TreeClipboardScope, TreeCommand, TreeCopyOccurrence, TreeDisclosure, TreeDisplayRow, TreeEvent,
    TreeProjection, TreePublication, TreeRefreshRequest, TreeRequestId, TreeRootSelection,
    TreeRootSnapshot, TreeSizeCommand, TreeSizeEvent,
};
use midnight_engine::Engine;
use midnight_platform::LocalFileSystem;
use std::{
    ffi::OsString,
    fs,
    os::unix::{
        ffi::OsStringExt,
        fs::{MetadataExt, symlink},
    },
    path::Path,
    sync::Arc,
    time::Duration,
};
use tokio::sync::mpsc;

const WAIT: Duration = Duration::from_secs(10);
const LOOP: &str = "00-ancestor-loop";
const READABLE: &str = "10-readable 文档";

fn owned_fixture() -> tempfile::TempDir {
    let build = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build");
    let fixtures = build.join("tmp/tree-link-native/linux");
    fs::create_dir_all(&fixtures).unwrap();
    let build = build.canonicalize().unwrap();
    let runtime_temp = std::env::temp_dir().canonicalize().unwrap();
    assert!(
        runtime_temp.is_absolute() && runtime_temp.starts_with(&build),
        "Set TMPDIR to an owned directory under {build:?} before this test"
    );
    let fixtures = fixtures.canonicalize().unwrap();
    assert!(
        fixtures.starts_with(&build),
        "Native fixtures must stay under _build"
    );
    tempfile::Builder::new()
        .prefix("tree-link-native-")
        .tempdir_in(fixtures)
        .unwrap()
}

async fn tree(commands: &mpsc::Sender<Command>, command: TreeCommand) {
    commands
        .send(Command::Tree(Box::new(command)))
        .await
        .unwrap();
}

async fn next_tree(events: &mut mpsc::Receiver<Event>) -> TreeEvent {
    tokio::time::timeout(WAIT, async {
        loop {
            match events.recv().await {
                Some(Event::Tree(event)) => return *event,
                Some(Event::ListingFailed { error, .. }) => {
                    panic!("native listing failed: {error}")
                }
                Some(_) => {}
                None => panic!("engine stopped before a tree response"),
            }
        }
    })
    .await
    .expect("native tree response must finish")
}

async fn publication(events: &mut mpsc::Receiver<Event>) -> TreePublication {
    match next_tree(events).await {
        TreeEvent::Published(publication) => publication,
        event => panic!("expected native tree publication: {event:?}"),
    }
}

fn row<'a>(view: &'a TreeProjection, path: &Path) -> &'a TreeDisplayRow {
    view.rows
        .iter()
        .find(|row| row.entry.native_path() == Some(path))
        .unwrap_or_else(|| panic!("missing logical native occurrence: {path:?}"))
}

async fn expand_alias(
    commands: &mpsc::Sender<Command>,
    events: &mut mpsc::Receiver<Event>,
    visible: TreePublication,
    alias: &Path,
    request: TreeRequestId,
) -> TreePublication {
    let branch = row(&visible.view, alias).id;
    tree(commands, TreeCommand::Commit(visible.view.clone())).await;
    tree(
        commands,
        TreeCommand::Branch {
            view: visible.view,
            request,
            row: branch,
            action: TreeBranchAction::ExpandRecursive,
        },
    )
    .await;
    // Commit every real publication; no fixed sleep substitutes for completion.
    // A cycle that kills the walk leaves this sibling unfinished and times out.
    tokio::time::timeout(WAIT, async {
        loop {
            let current = publication(events).await;
            let ready = current.view.rows.iter().any(|row| {
                row.entry.native_path() == Some(alias.join(READABLE).join("leaf").as_path())
                    && row.disclosure == TreeDisclosure::Empty
            });
            tree(commands, TreeCommand::Commit(current.view.clone())).await;
            if ready {
                return current;
            }
        }
    })
    .await
    .expect("readable sibling must finish after the ancestor loop is rejected")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_symlink_tree_keeps_alias_actions_and_rejects_only_ancestor_loops() {
    let owned = owned_fixture();
    let root = owned.path().join("root 資料");
    let target = owned.path().join("external target & literal");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&target).unwrap();
    symlink(&root, target.join(LOOP)).unwrap();
    fs::create_dir_all(target.join(READABLE).join("leaf")).unwrap();
    let sentinel = target.join("sentinel.txt");
    fs::write(&sentinel, b"unchanged target contents").unwrap();
    let aliases = [
        root.join("10-alias e\u{301}"),
        root.join(OsString::from_vec(vec![b'2', b'0', b'-', b'a', 0xff])),
    ];
    for alias in &aliases {
        symlink(&target, alias).unwrap();
    }

    let filesystem = Arc::new(LocalFileSystem::new(None));
    let cancel = Cancellation::default();
    let root_target = filesystem
        .resolve_size_directory(&root, &cancel)
        .unwrap()
        .unwrap();
    let resolved_target = filesystem
        .resolve_size_directory(&target, &cancel)
        .unwrap()
        .unwrap();
    let native_metadata = fs::metadata(&target).unwrap();
    assert_eq!(resolved_target.identity.device, native_metadata.dev());
    assert_eq!(
        resolved_target.identity.object,
        u128::from(native_metadata.ino()).to_le_bytes()
    );
    assert_ne!(root_target.identity, resolved_target.identity);
    for alias in &aliases {
        let resolved = filesystem
            .resolve_size_directory(alias, &cancel)
            .unwrap()
            .unwrap();
        assert_eq!(resolved.identity, resolved_target.identity);
        assert_eq!(resolved.path, resolved_target.path);
        assert_eq!(
            filesystem
                .browse_directory_identity(alias, &cancel)
                .unwrap(),
            Some(resolved.identity),
            "the browse hook preserves the complete native device/inode identity"
        );
        let ancestor = filesystem
            .resolve_size_directory(&alias.join(LOOP), &cancel)
            .unwrap()
            .unwrap();
        assert_eq!(ancestor.identity, root_target.identity);
    }
    assert_eq!(
        filesystem
            .browse_directory_identity(&sentinel, &cancel)
            .unwrap(),
        None
    );

    let (commands, receive) = mpsc::channel(16);
    let (send, mut events) = mpsc::channel(32);
    let engine =
        tokio::spawn(Engine::new(Engine::lifetime().unwrap(), filesystem, send).run(receive));
    commands
        .send(Command::Navigate {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            path: root.clone(),
            sort: SortSpec::default(),
        })
        .await
        .unwrap();
    let snapshot = tokio::time::timeout(WAIT, async {
        loop {
            match events.recv().await {
                Some(Event::ListingFinished {
                    pane,
                    navigation,
                    entries,
                    index,
                }) => {
                    assert_eq!((pane, navigation), (PaneId::Left, NavigationId(1)));
                    for alias in &aliases {
                        let entry = entries
                            .iter()
                            .find(|entry| entry.native_path() == Some(alias.as_path()))
                            .unwrap();
                        assert_eq!(entry.kind, EntryKind::DirectoryLink);
                        assert_eq!(entry.name, alias.file_name().unwrap());
                    }
                    return TreeRootSnapshot {
                        pane,
                        navigation,
                        entries,
                        index,
                    };
                }
                Some(Event::ListingFailed { error, .. }) => panic!("native root failed: {error}"),
                Some(_) => {}
                None => panic!("engine stopped before native root capture"),
            }
        }
    })
    .await
    .expect("native root listing must finish");
    tree(
        &commands,
        TreeCommand::Enable {
            root: snapshot,
            selection_revision: SelectionRevision(0),
            selected: TreeRootSelection::Empty,
        },
    )
    .await;
    let mut visible = publication(&mut events).await;
    for (index, alias) in aliases.iter().enumerate() {
        visible = expand_alias(
            &commands,
            &mut events,
            visible,
            alias,
            TreeRequestId(index as u64 + 1),
        )
        .await;
        let parent = row(&visible.view, alias);
        assert_eq!(parent.entry.kind, EntryKind::DirectoryLink);
        assert_eq!(parent.disclosure, TreeDisclosure::Expanded);
        let readable = row(&visible.view, &alias.join(READABLE));
        assert_eq!(readable.parent, Some(parent.id));
        assert_eq!(readable.disclosure, TreeDisclosure::Expanded);
        let leaf = row(&visible.view, &alias.join(READABLE).join("leaf"));
        assert_eq!(leaf.parent, Some(readable.id));
        assert_eq!(leaf.disclosure, TreeDisclosure::Empty);
        let cycle = row(&visible.view, &alias.join(LOOP));
        assert_eq!(cycle.entry.kind, EntryKind::DirectoryLink);
        assert_eq!(cycle.disclosure, TreeDisclosure::Failed);
        assert!(
            cycle
                .error
                .as_deref()
                .is_some_and(|message| message.contains("ancestor"))
        );
        assert!(
            !visible
                .view
                .rows
                .iter()
                .any(|row| row.parent == Some(cycle.id))
        );
    }
    assert_eq!(
        visible.view.rows.len(),
        8,
        "both sibling aliases retain their own children"
    );

    // Exercise the actual captured-action API without publishing to an OS clipboard.
    // Resolver target paths are browse observations, never substitute action paths.
    for (index, alias) in aliases.iter().enumerate() {
        let logical = alias.join(READABLE);
        let occurrence = row(&visible.view, &logical).id;
        let request = TreeRequestId(index as u64 + 3);
        tree(
            &commands,
            TreeCommand::Clipboard(TreeClipboardRequest {
                scope: TreeClipboardScope::SelectionOrFallback,
                intent: FileClipboardIntent::Copy,
                view: visible.view.clone(),
                request,
                selection_revision: visible.selection_revision,
                selection: visible.selection.clone(),
                fallback: Some(occurrence),
                cancel: Cancellation::default(),
            }),
        )
        .await;
        let source = match next_tree(&mut events).await {
            TreeEvent::Clipboard {
                request: actual,
                result,
                ..
            } => {
                assert_eq!(actual, request);
                result.expect("logical native child must be capturable")
            }
            event => panic!("expected captured native child: {event:?}"),
        };
        assert_eq!(
            source.tree_copy_occurrence(0),
            Some(TreeCopyOccurrence {
                stamp: visible.view.stamp,
                row: occurrence
            })
        );
        let files =
            tokio::task::spawn_blocking(move || source.resolve(&Cancellation::default()).unwrap())
                .await
                .unwrap();
        assert_eq!(files.intent, FileClipboardIntent::Copy);
        assert_eq!(files.paths, [logical]);
        assert_ne!(files.paths[0], resolved_target.path.join(READABLE));
    }

    // Refresh the actual target through both loaded logical symlink occurrences.
    let before_refresh = visible.view.clone();
    let added = "after refresh 文档";
    fs::create_dir(target.join(added)).unwrap();
    fs::create_dir(target.join(READABLE).join("leaf").join(added)).unwrap();
    let request = TreeRefreshRequest {
        view: visible.view.clone(),
        request: TreeRequestId(5),
    };
    tree(&commands, TreeCommand::Commit(request.view.clone())).await;
    tree(&commands, TreeCommand::RefreshLoaded(request.clone())).await;
    tokio::time::timeout(WAIT, async {
        loop {
            match next_tree(&mut events).await {
                TreeEvent::Published(publication) => {
                    tree(&commands, TreeCommand::Commit(publication.view.clone())).await;
                    visible = publication;
                }
                TreeEvent::Refreshed {
                    stamp,
                    request: id,
                    result,
                } => {
                    assert_eq!(stamp, request.view.stamp);
                    assert_eq!(id, request.request);
                    result.expect("readable native symlink refresh must succeed");
                    break;
                }
                TreeEvent::Sizes(TreeSizeEvent::Snapshot { sizes }) => {
                    assert_eq!(sizes.stamp.session, request.view.stamp.session);
                    assert!(sizes.is_empty());
                    tree(
                        &commands,
                        TreeCommand::Sizes(TreeSizeCommand::Acknowledge {
                            stamp: sizes.stamp,
                            version: sizes.version,
                        }),
                    )
                    .await;
                }
                TreeEvent::Sizes(TreeSizeEvent::Selected {
                    view, selection, ..
                }) => {
                    assert_eq!(view.stamp.session, request.view.stamp.session);
                    assert!(Arc::ptr_eq(&view, selection.basis()));
                }
                event => panic!("unexpected native refresh event: {event:?}"),
            }
        }
    })
    .await
    .expect("native symlink refresh must complete");
    assert_eq!(visible.view.stamp.session, before_refresh.stamp.session);
    for alias in &aliases {
        let parent = row(&visible.view, alias);
        assert_eq!(parent.id, row(&before_refresh, alias).id);
        assert_eq!(parent.disclosure, TreeDisclosure::Expanded);
        assert_eq!(
            row(&visible.view, &alias.join(added)).parent,
            Some(parent.id)
        );
        let leaf_path = alias.join(READABLE).join("leaf");
        let leaf = row(&visible.view, &leaf_path);
        assert_eq!(leaf.id, row(&before_refresh, &leaf_path).id);
        assert_eq!(leaf.disclosure, TreeDisclosure::Expanded);
        assert_eq!(
            row(&visible.view, &leaf_path.join(added)).parent,
            Some(leaf.id)
        );
        let cycle = row(&visible.view, &alias.join(LOOP));
        assert_eq!(cycle.disclosure, TreeDisclosure::Failed);
        assert!(
            !visible
                .view
                .rows
                .iter()
                .any(|row| row.parent == Some(cycle.id))
        );
    }
    assert_eq!(
        visible.view.rows.len(),
        12,
        "refresh must update both sibling aliases"
    );
    commands.send(Command::Shutdown).await.unwrap();
    tokio::time::timeout(WAIT, engine).await.unwrap().unwrap();
    assert_eq!(fs::read(&sentinel).unwrap(), b"unchanged target contents");
}
