//! Preserve both pane registries and current roots under archive cleanup admission.
use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn both_panes_retain_full_archive_registries_and_admit_replacement_roots() {
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
    let mut retained = Vec::new();
    // The existing public behavior is 64 retained mounts in each pane registry.
    // These are independent source identities, not aliases sharing a native guard.
    for generation in 1..=64 {
        for pane in [PaneId::Left, PaneId::Right] {
            let root = archive_root_in_pane_at(
                &commands,
                &mut events,
                Arc::new(Archive),
                pane,
                generation,
            )
            .await;
            let publication = enable(&commands, &mut events, root).await;
            assert_eq!(publication.view.stamp.pane, pane);
            tree(&commands, TreeCommand::Commit(publication.view.clone())).await;
            tree(
                &commands,
                TreeCommand::Navigate {
                    view: publication.view.clone(),
                    request: TreeRequestId(1),
                    row: publication.view.rows[0].id,
                    navigation: NavigationId(generation + 1),
                },
            )
            .await;
            let TreeEvent::Navigated { target, .. } = next_tree(&mut events).await else {
                panic!("navigation target expected");
            };
            retained.push(target);
        }
    }
    let mut roots = Vec::new();
    for pane in [PaneId::Left, PaneId::Right] {
        let root =
            archive_root_in_pane_at(&commands, &mut events, Arc::new(Archive), pane, 65).await;
        assert_eq!(root.pane, pane);
        tree(
            &commands,
            TreeCommand::Enable {
                root: root.clone(),
                selection_revision: SelectionRevision(0),
                selected: TreeRootSelection::Empty,
            },
        )
        .await;
        let limited = next_tree(&mut events).await;
        assert!(
            matches!(limited, TreeEvent::Unavailable { .. }),
            "full tree registry must still refuse its 65th mount: {limited:?}"
        );
        roots.push(root);
    }
    // Releasing consumers restores each independent registry; neither current
    // archive root was evicted or refused by the aggregate archive retention bound.
    drop(retained);
    for root in roots {
        let pane = root.pane;
        let publication = enable(&commands, &mut events, root).await;
        assert_eq!(publication.view.stamp.pane, pane);
        assert_eq!(publication.view.stamp.navigation, NavigationId(65));
    }
    commands.send(Command::Shutdown).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), engine)
        .await
        .unwrap()
        .unwrap();
}
