//! Opt-in read-only qualification with the installed native provider and a real folder.
use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a protected native plugin installation and explicit read-only test path"]
async fn installed_native_expansion_and_navigation_survive_viewport_pressure() {
    let installation = std::env::var_os("MIDNIGHT_NATIVE_TEST_INSTALLATION")
        .expect("Set protected native plugin installation");
    let parent = std::env::var_os("MIDNIGHT_TREE_TEST_PARENT").expect("Set parent directory");
    let child = std::env::var_os("MIDNIGHT_TREE_TEST_CHILD").expect("Set child directory name");
    let (commands, receive) = mpsc::channel(16);
    let (send, mut events) = mpsc::channel(32);
    let provider = midnight_native_filesystem_host::InstalledProvider::from_installation(
        installation.into(),
        None,
    );
    let engine = tokio::spawn(
        Engine::new(Engine::lifetime().unwrap(), Arc::new(provider), send).run(receive),
    );
    let root = native_root_at(&commands, &mut events, 1, parent.into()).await;
    let initial = enable(&commands, &mut events, root).await;
    let target = initial
        .view
        .rows
        .iter()
        .find(|row| row.entry.name == child)
        .expect("child directory must be present in captured listing");
    let row = target.id;
    let expected = target.entry.path.clone();
    tree(&commands, TreeCommand::Commit(initial.view.clone())).await;
    tree(
        &commands,
        TreeCommand::Branch {
            view: initial.view,
            request: TreeRequestId(1),
            row,
            action: TreeBranchAction::Expand,
        },
    )
    .await;
    let expanded = loop {
        let publication = published(&mut events).await;
        tree(&commands, TreeCommand::Commit(publication.view.clone())).await;
        let folder = &publication.view.rows[publication.view.index.row_positions[&row]];
        assert!(
            folder.error.is_none(),
            "selected folder cannot be expanded: {:?}",
            folder.error
        );
        if folder.disclosure != TreeDisclosure::Loading {
            assert!(matches!(
                folder.disclosure,
                TreeDisclosure::Expanded | TreeDisclosure::Empty
            ));
            break publication.view;
        }
    };
    let children = expanded
        .rows
        .iter()
        .filter(|child| child.parent == Some(row))
        .take(16)
        .map(|child| child.id)
        .collect::<Vec<_>>();
    for _ in 0..64 {
        tree(
            &commands,
            TreeCommand::Viewport {
                view: expanded.clone(),
                rows: TreeViewport::new(&expanded, children.clone()).unwrap(),
            },
        )
        .await;
    }
    tree(
        &commands,
        TreeCommand::Navigate {
            view: expanded,
            request: TreeRequestId(2),
            row,
            navigation: NavigationId(2),
        },
    )
    .await;
    let mut observed_errors = 0;
    loop {
        match next_tree(&mut events).await {
            TreeEvent::Published(publication) => {
                observed_errors += publication
                    .view
                    .rows
                    .iter()
                    .filter(|row| row.error.is_some())
                    .count();
                tree(&commands, TreeCommand::Commit(publication.view)).await;
            }
            TreeEvent::Navigated {
                request: TreeRequestId(2),
                target,
                ..
            } => {
                let TreeNavigationTarget::Native(path) = target else {
                    panic!("native folder must retain its native target");
                };
                assert_eq!(path, expected);
                break;
            }
            event => panic!("explicit navigation must survive background probes: {event:?}"),
        }
    }
    eprintln!(
        "Expanded and navigated {} under 64 viewport observations; child diagnostics observed: {observed_errors}",
        expected.display()
    );
    commands.send(Command::Shutdown).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), engine)
        .await
        .unwrap()
        .unwrap();
}
