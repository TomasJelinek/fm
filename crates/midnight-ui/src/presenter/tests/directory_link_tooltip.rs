use super::*;

#[test]
fn directory_link_tooltip_matches_the_hover_and_current_listing() {
    let root = test_root().join("link-tooltip");
    let mut p = Presenter::new(root.clone(), root.clone());
    let pane = PaneId::Left;
    let generation = begin(&mut p, pane, root.to_str().unwrap());
    let link = Entry::new(
        EntryId(11),
        root.join("linked folder"),
        EntryKind::DirectoryLink,
    );
    let regular = Entry::new(EntryId(12), root.join("ordinary"), EntryKind::Directory);
    finish(
        &mut p,
        pane,
        generation,
        vec![link.clone(), regular.clone()],
    );

    let link_identity = p.panes[pane.index()].row_identity(&link);
    let regular_identity = p.panes[pane.index()].row_identity(&regular);
    p.read_directory_link_target(pane, &regular_identity, &|_| {
        panic!("a regular directory has no stored link target");
    });
    assert!(p.directory_link_tooltip().is_none());

    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    p.read_directory_link_target(pane, &link_identity, &send);
    p.read_directory_link_target(pane, &link_identity, &send);
    let captured = commands.borrow();
    let [
        Command::ReadDirectoryLinkTarget {
            request,
            pane: command_pane,
            navigation,
            path,
        },
    ] = captured.as_slice()
    else {
        panic!("one lookup for the hovered link")
    };
    assert_eq!(
        (*command_pane, *navigation, path),
        (pane, generation, &link.path)
    );
    let request = *request;
    drop(captured);
    let reply = |request, navigation, target| Event::DirectoryLinkTargetRead {
        request,
        pane,
        navigation,
        target,
    };

    p.apply(reply(request, NavigationId(generation.0 + 1), None));
    assert_eq!(
        p.directory_link_tooltip().unwrap().2,
        "Reading link target…"
    );
    p.directory_link_hover_changed(pane, &regular_identity);
    p.apply(reply(
        request,
        generation,
        Some(PathBuf::from("old target")),
    ));
    assert!(p.directory_link_tooltip().is_none());

    p.read_directory_link_target(pane, &link_identity, &send);
    let new_request = match &commands.borrow()[1] {
        Command::ReadDirectoryLinkTarget { request, .. } => *request,
        other => panic!("unexpected hover command {other:?}"),
    };
    assert_ne!(new_request, request);
    p.apply(reply(
        request,
        generation,
        Some(PathBuf::from("old target")),
    ));
    assert_eq!(
        p.directory_link_tooltip().unwrap().2,
        "Reading link target…"
    );
    p.apply(reply(
        new_request,
        generation,
        Some(PathBuf::from("../stored target")),
    ));
    assert_eq!(
        p.directory_link_tooltip().unwrap().2,
        "Link target\n../stored target",
    );

    p.directory_link_hover_changed(pane, "");
    p.read_directory_link_target(pane, &link_identity, &|_| false);
    assert_eq!(
        p.directory_link_tooltip().unwrap().2,
        "Link target unavailable"
    );
    begin(&mut p, pane, root.join("another").to_str().unwrap());
    assert!(p.directory_link_tooltip().is_none());
}

#[test]
fn inline_directory_link_lookup_fences_a_replaced_tree_projection() {
    let (mut p, root) = super::tree::fixture();
    let expected_path = root.entries[0].path.join("child");
    let publish = |p: &mut Presenter, revision| {
        let base = super::tree::publication(root.clone(), revision, true, false);
        let mut rows = base.view.rows.to_vec();
        rows[1].entry.kind = EntryKind::DirectoryLink;
        rows[1].disclosure = TreeDisclosure::None;
        let view = Arc::new(
            TreeProjection::new(
                root.clone(),
                base.view.stamp,
                TreeLayoutRevision(revision),
                rows,
            )
            .unwrap(),
        );
        let selection = Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![0]).unwrap());
        let publication = TreePublication::new(view, SelectionRevision(0), selection).unwrap();
        p.apply_with_sender(
            Event::Tree(Box::new(TreeEvent::Published(publication))),
            &|_| true,
        );
    };
    publish(&mut p, 1);

    let first = p.panes[0].row_identity(&p.panes[0].tree_row(1).unwrap().entry);
    let sends = RefCell::new(Vec::new());
    let send = |command| {
        sends.borrow_mut().push(command);
        true
    };
    p.read_directory_link_target(PaneId::Left, &first, &send);
    let first_request = match &sends.borrow()[0] {
        Command::ReadDirectoryLinkTarget {
            request,
            pane,
            navigation,
            path,
        } => {
            assert_eq!(
                (*pane, *navigation, path),
                (PaneId::Left, root.navigation, &expected_path)
            );
            *request
        }
        other => panic!("unexpected inline link command {other:?}"),
    };

    publish(&mut p, 2);
    p.apply(Event::DirectoryLinkTargetRead {
        request: first_request,
        pane: PaneId::Left,
        navigation: root.navigation,
        target: Some(PathBuf::from("stale target")),
    });
    assert!(p.directory_link_tooltip().is_none());
    let current = p.panes[0].row_identity(&p.panes[0].tree_row(1).unwrap().entry);
    assert_ne!(first, current);
    p.read_directory_link_target(PaneId::Left, &current, &send);
    let current_request = match &sends.borrow()[1] {
        Command::ReadDirectoryLinkTarget { request, .. } => *request,
        other => panic!("unexpected replacement link command {other:?}"),
    };
    p.apply(Event::DirectoryLinkTargetRead {
        request: current_request,
        pane: PaneId::Left,
        navigation: root.navigation,
        target: None,
    });
    assert_eq!(
        p.directory_link_tooltip().unwrap().2,
        "Link target unavailable"
    );
}
