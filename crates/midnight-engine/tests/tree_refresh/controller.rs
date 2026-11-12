use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_sweep_publishes_actual_additions_and_removals_before_correlated_completion() {
    let directory = tempfile::tempdir().unwrap();
    let parent = directory.path().join("parent");
    std::fs::create_dir(&parent).unwrap();
    std::fs::create_dir(parent.join("old child")).unwrap();
    std::fs::write(directory.path().join("old.txt"), b"old").unwrap();
    let (commands, receive) = mpsc::channel(16);
    let (send, mut events) = mpsc::channel(32);
    let engine = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            Arc::new(midnight_platform::LocalFileSystem::new(None)),
            send,
        )
        .run(receive),
    );
    let root = native_root_at(&commands, &mut events, 1, directory.path().to_owned()).await;
    let first = enable(&commands, &mut events, root).await;
    let parent_row = first
        .view
        .rows
        .iter()
        .find(|row| row.entry.display_name == "parent")
        .unwrap()
        .id;
    tree(&commands, TreeCommand::Commit(first.view.clone())).await;
    tree(
        &commands,
        TreeCommand::Branch {
            view: first.view.clone(),
            request: TreeRequestId(1),
            row: parent_row,
            action: TreeBranchAction::Expand,
        },
    )
    .await;
    let expanded = loop {
        let publication = published(&mut events).await;
        if publication
            .view
            .rows
            .iter()
            .any(|row| row.entry.display_name == "old child")
        {
            break publication;
        }
    };
    tree(&commands, TreeCommand::Commit(expanded.view.clone())).await;
    std::fs::remove_dir(parent.join("old child")).unwrap();
    std::fs::create_dir(parent.join("new child")).unwrap();
    std::fs::remove_file(directory.path().join("old.txt")).unwrap();
    std::fs::write(directory.path().join("new.txt"), b"new").unwrap();
    let request = TreeRefreshRequest {
        view: expanded.view.clone(),
        request: TreeRequestId(2),
    };
    tree(&commands, TreeCommand::RefreshLoaded(request.clone())).await;
    let mut latest = expanded.view;
    let mut publications = 0;
    loop {
        match next_tree(&mut events).await {
            TreeEvent::Published(publication) => {
                latest = publication.view;
                publications += 1;
            }
            TreeEvent::Refreshed {
                stamp,
                request: id,
                result,
            } => {
                assert_eq!(stamp, request.view.stamp);
                assert_eq!(id, request.request);
                assert!(result.is_ok(), "{result:?}");
                break;
            }
            TreeEvent::Sizes(TreeSizeEvent::Snapshot { sizes }) => {
                assert_eq!(sizes.stamp.session, request.view.stamp.session);
                assert!(
                    sizes.is_empty(),
                    "maintenance invalidates previous folder sizes"
                );
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
            event => panic!("unexpected maintenance event: {event:?}"),
        }
    }
    assert!(publications >= 2);
    assert_eq!(latest.stamp.session, first.view.stamp.session);
    assert!(latest.root.same_snapshot(&first.view.root));
    assert_eq!(
        latest
            .rows
            .iter()
            .map(|row| row.entry.display_name.as_str())
            .collect::<Vec<_>>(),
        ["parent", "new child", "new.txt"]
    );
    assert_eq!(latest.rows[0].id, parent_row);
    assert!(latest.rows[0].expanded);
    let alias = latest.rows[2].original_root_entry.unwrap();
    assert_eq!(
        latest.root_actions.entries[latest.root_actions.index.by_id[&alias]].path,
        directory.path().join("new.txt")
    );
    tree(&commands, TreeCommand::RefreshLoaded(request.clone())).await;
    assert!(
        matches!(next_tree(&mut events).await, TreeEvent::RefreshRejected { stamp, request: id, retryable: true, .. } if stamp == request.view.stamp && id == request.request)
    );
    commands.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn archive_refresh_is_correlated_unsupported_and_preserves_initial_action_snapshot() {
    let (commands, receive) = mpsc::channel(16);
    let (send, mut events) = mpsc::channel(32);
    let engine = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            Arc::new(Files),
            send,
        )
        .run(receive),
    );
    let root = archive_root(&commands, &mut events, Arc::new(Archive)).await;
    let publication = enable(&commands, &mut events, root).await;
    assert!(
        publication
            .view
            .root
            .same_snapshot(&publication.view.root_actions)
    );
    tree(&commands, TreeCommand::Commit(publication.view.clone())).await;
    tree(
        &commands,
        TreeCommand::RefreshLoaded(TreeRefreshRequest {
            view: publication.view.clone(),
            request: TreeRequestId(1),
        }),
    )
    .await;
    assert!(
        matches!(next_tree(&mut events).await, TreeEvent::RefreshRejected { stamp, request: TreeRequestId(1), retryable: false, .. } if stamp == publication.view.stamp)
    );
    commands.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
}
