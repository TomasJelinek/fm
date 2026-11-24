use super::*;

fn space(available: u64) -> SpaceInfo {
    SpaceInfo {
        available,
        total: 1000,
    }
}

/// Left shows a folder on volume C, right one on volume D.
fn two_volumes() -> (Presenter, PathBuf, PathBuf) {
    let c = test_root().join("volume-c");
    let d = test_root().join("volume-d");
    let mut p = Presenter::new(c.join("shown"), d.join("shown"));
    p.apply(Event::VolumesUpdated(vec![
        Volume {
            id: "c".into(),
            label: "C:".into(),
            root: c.clone(),
        },
        Volume {
            id: "d".into(),
            label: "D:".into(),
            root: d.clone(),
        },
        // The Home location is a folder of volume C, not a volume.
        Volume {
            id: "home".into(),
            label: "Home".into(),
            root: c.join("shown"),
        },
    ]));
    for (pane, path) in [
        (PaneId::Left, c.join("shown")),
        (PaneId::Right, d.join("shown")),
    ] {
        let navigation = begin(&mut p, pane, path.to_str().unwrap());
        finish(&mut p, pane, navigation, vec![]);
    }
    (p, c, d)
}

/// The panes, with their navigations, whose free space `event` asks the backend
/// to read again.
fn requested_for(p: &mut Presenter, event: Event) -> Vec<(PaneId, NavigationId)> {
    let sent = RefCell::new(Vec::new());
    p.apply_with_sender(event, &|command| {
        if let Command::RefreshSpace { pane, navigation } = command {
            sent.borrow_mut().push((pane, navigation));
        }
        true
    });
    sent.into_inner()
}

fn shown(p: &Presenter, pane: PaneId) -> (PaneId, NavigationId) {
    let navigation = p.panes[pane.index()].successful_generation.unwrap();
    (pane, navigation)
}

fn copy(p: &mut Presenter, intent: TransferIntent, destination: PathBuf) -> CopyProgress {
    let progress = CopyProgress::queued(OperationId(7), destination, 1, intent);
    p.copies.progress.push(Arc::new(progress.clone()));
    progress
}

#[test]
fn a_copy_to_another_folder_of_a_panels_volume_asks_for_its_free_space_until_it_ends() {
    let (mut p, c, d) = two_volumes();
    let mut progress = copy(&mut p, TransferIntent::Copy, c.join("elsewhere"));
    // A queued job has not written anything yet.
    assert!(
        requested_for(
            &mut p,
            Event::Copy(CopyEvent::Progress(Arc::new(progress.clone())))
        )
        .is_empty()
    );

    progress.state = CopyOperationState::Running;
    let left = shown(&p, PaneId::Left);
    assert_eq!(
        requested_for(
            &mut p,
            Event::Copy(CopyEvent::Progress(Arc::new(progress.clone())))
        ),
        [left]
    );
    progress.state = CopyOperationState::Completed;
    assert_eq!(
        requested_for(&mut p, Event::Copy(CopyEvent::Progress(Arc::new(progress)))),
        [left]
    );

    // A Move also frees space on its sources' volume.
    let mut moved = CopyProgress::queued(OperationId(8), c.join("target"), 1, TransferIntent::Move);
    p.copies.progress.push(Arc::new(moved.clone()));
    moved.state = CopyOperationState::Running;
    moved.source_directories = vec![d.join("source")];
    let right = shown(&p, PaneId::Right);
    assert_eq!(
        requested_for(&mut p, Event::Copy(CopyEvent::Progress(Arc::new(moved)))),
        [left, right]
    );
}

#[test]
fn a_delete_asks_for_the_free_space_of_the_panels_on_its_volume() {
    let (mut p, _, d) = two_volumes();
    let progress = OperationProgress {
        pause_requested: false,
        paused: false,
        discovery: Default::default(),
        id: OperationId(3),
        kind: MutationKind::Delete(DeleteMode::Permanent),
        state: OperationState::Running,
        total: 1,
        completed: 0,
        failed: 0,
        processed_entries: 0,
        throughput: Arc::default(),
        current_path: None,
        message: String::new(),
        errors: vec![],
        source_directory: d.join("shown").join("old"),
        renamed_to: None,
    };
    let right = shown(&p, PaneId::Right);
    assert_eq!(
        requested_for(
            &mut p,
            Event::Operation(OperationEvent::Progress(Arc::new(progress)))
        ),
        [right]
    );
}

#[test]
fn a_loading_panel_asks_once_its_listing_committed_and_a_new_location_forgets_it() {
    let (mut p, c, _) = two_volumes();
    let shown = p.panes[0].successful_generation.unwrap();
    assert!(p.refresh_if_changed(PaneId::Left, shown, &|_| true));
    let reread = NavigationId(p.panes[0].generation);
    let mut progress = copy(&mut p, TransferIntent::Copy, c.join("elsewhere"));
    progress.state = CopyOperationState::Running;
    // The listing's own read may have preceded this write.
    assert!(
        requested_for(
            &mut p,
            Event::Copy(CopyEvent::Progress(Arc::new(progress.clone())))
        )
        .is_empty()
    );
    let index = Arc::new(DirectoryIndex::from_entries(&[]));
    assert_eq!(
        requested_for(
            &mut p,
            Event::ListingFinished {
                pane: PaneId::Left,
                navigation: reread,
                entries: Vec::new().into(),
                index,
            }
        ),
        [(PaneId::Left, reread)]
    );

    // Owed for a folder the panel then left: its new listing reads the new volume.
    assert!(
        requested_for(
            &mut p,
            Event::Copy(CopyEvent::Progress(Arc::new(progress.clone())))
        )
        .len()
            == 1
    );
    p.panes[0].loading = true;
    assert!(requested_for(&mut p, Event::Copy(CopyEvent::Progress(Arc::new(progress)))).is_empty());
    p.panes[0].loading = false;
    let other = begin(
        &mut p,
        PaneId::Left,
        test_root().join("volume-d").to_str().unwrap(),
    );
    finish(&mut p, PaneId::Left, other, vec![]);
    let sent = RefCell::new(0);
    p.flush_space_refreshes(&|_| {
        *sent.borrow_mut() += 1;
        true
    });
    assert_eq!(sent.into_inner(), 0);
}

#[test]
fn focus_asks_both_panels_and_a_visibility_change_is_sent_once() {
    let (mut p, _, _) = two_volumes();
    let sent = RefCell::new(Vec::new());
    let record = |command: Command| {
        sent.borrow_mut().push(match command {
            Command::RefreshSpace { pane, .. } => format!("refresh {pane:?}"),
            Command::SetWindowVisible { visible } => format!("visible {visible}"),
            _ => return true,
        });
        true
    };
    p.window_presence(true, false, &record);
    assert!(sent.borrow().is_empty());
    p.window_presence(false, false, &record);
    p.window_presence(false, false, &record);
    p.window_presence(true, true, &record);
    assert_eq!(
        sent.into_inner(),
        [
            "visible false",
            "visible true",
            "refresh Left",
            "refresh Right"
        ]
    );
    // A full command queue keeps the change for the next event.
    let refused = |_: Command| false;
    p.window_presence(false, false, &refused);
    let sent = RefCell::new(Vec::new());
    p.window_presence(false, false, &|command| {
        sent.borrow_mut().push(matches!(
            command,
            Command::SetWindowVisible { visible: false }
        ));
        true
    });
    assert_eq!(sent.into_inner(), [true]);
}

#[test]
fn a_changed_value_changes_the_panel_text_and_an_equal_one_changes_nothing() {
    let (mut p, _, _) = two_volumes();
    let shown = p.panes[0].successful_generation.unwrap();
    let text = |p: &Presenter| crate::volume_presentation::panel_space(p, PaneId::Left);
    for (available, expected) in [
        (500, "500 B free"),
        (400, "400 B free"),
        (400, "400 B free"),
    ] {
        drop(p.take_retired());
        p.apply(Event::SpaceUpdated {
            pane: PaneId::Left,
            navigation: shown,
            space: space(available),
        });
        assert_eq!(text(&p), expected);
        // Nothing was replaced; the window's text property ignores an equal value.
        assert!(p.take_retired().is_empty());
    }
}
