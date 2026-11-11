//! Automatic tree observations may arrive after a faster provider publishes a new view.
use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn files_only_expansion_ignores_superseded_commit_and_viewport() {
    let directory = tempfile::tempdir().unwrap();
    let bin = directory.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    for name in ["bash.exe", "git.exe", "sh.exe"] {
        std::fs::write(bin.join(name), b"fixture").unwrap();
    }
    assert_leaf_expansion(
        Arc::new(midnight_platform::LocalFileSystem::new(None)),
        directory.path(),
        "bin",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires protected native plugin installation and a real files-only child folder"]
async fn installed_native_files_only_expansion_ignores_superseded_observations() {
    let installation = std::env::var_os("MIDNIGHT_NATIVE_TEST_INSTALLATION")
        .expect("Set protected native plugin installation");
    let parent = std::env::var_os("MIDNIGHT_TREE_TEST_PARENT").expect("Set parent directory");
    let child = std::env::var("MIDNIGHT_TREE_TEST_CHILD").expect("Set files-only child name");
    assert_leaf_expansion(
        Arc::new(
            midnight_native_filesystem_host::InstalledProvider::from_installation(
                installation.into(),
                None,
            ),
        ),
        Path::new(&parent),
        &child,
    )
    .await;
}

async fn assert_leaf_expansion(filesystem: Arc<dyn FileSystem>, parent: &Path, child: &str) {
    let (commands, receive) = mpsc::channel(16);
    let (send, mut events) = mpsc::channel(32);
    let engine = tokio::spawn(Engine::new(filesystem, send).run(receive));
    let root = native_root_at(&commands, &mut events, 1, parent.to_owned()).await;
    let initial = enable(&commands, &mut events, root).await;
    let position = initial
        .view
        .rows
        .iter()
        .position(|row| row.entry.name == child)
        .expect("child directory");
    let row = initial.view.rows[position].id;
    tree(&commands, TreeCommand::Commit(initial.view.clone())).await;
    tree(
        &commands,
        TreeCommand::Branch {
            view: initial.view.clone(),
            request: TreeRequestId(1),
            row,
            action: TreeBranchAction::Expand,
        },
    )
    .await;
    let loading = published(&mut events).await;
    assert_eq!(
        loading.view.rows[position].disclosure,
        TreeDisclosure::Loading
    );
    let empty = published(&mut events).await;
    assert_eq!(empty.view.rows.len(), initial.view.rows.len());
    assert_eq!(empty.view.rows[position].disclosure, TreeDisclosure::Empty);
    assert!(!empty.view.rows[position].expanded);
    assert!(empty.view.rows[position].error.is_none());

    // Delivery can lag publication: the UI still sends observations for either
    // the initial or loading view. Neither observation is a failed user action.
    for old in [initial.view.clone(), loading.view] {
        tree(&commands, TreeCommand::Commit(old.clone())).await;
        let rows = TreeViewport::new(&old, vec![row]).unwrap();
        tree(&commands, TreeCommand::Viewport { view: old, rows }).await;
    }
    tree(&commands, TreeCommand::Commit(empty.view.clone())).await;
    tree(
        &commands,
        TreeCommand::Select(TreeSelectionRequest {
            view: empty.view.clone(),
            revision: SelectionRevision(1),
            seed: None,
            seed_revision: SelectionRevision(0),
            action: TreeSelectionAction::Toggle(row),
        }),
    )
    .await;
    // Selection is a barrier through both command queues: there must be no
    // stale-observation rejection before it, and the current commit must work.
    let event = next_tree(&mut events).await;
    assert!(
        matches!(event, TreeEvent::SelectionChanged { .. }),
        "{event:?}"
    );

    // Explicit actions are still fenced; never execute one against a stale view.
    tree(
        &commands,
        TreeCommand::Branch {
            view: initial.view,
            request: TreeRequestId(2),
            row,
            action: TreeBranchAction::Expand,
        },
    )
    .await;
    assert!(matches!(
        next_tree(&mut events).await,
        TreeEvent::Rejected {
            request: TreeRequestId(2),
            ..
        }
    ));
    commands.send(Command::Shutdown).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), engine)
        .await
        .unwrap()
        .unwrap();
}
