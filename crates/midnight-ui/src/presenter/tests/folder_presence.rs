use super::*;

#[path = "folder_presence_cache.rs"]
mod cache;

fn ready() -> Presenter {
    let path = test_root().join("flat-presence");
    let mut p = Presenter::new(path.clone(), path.clone());
    let navigation = begin(&mut p, PaneId::Left, path.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        vec![
            Entry::new(EntryId(1), path.join("empty"), EntryKind::Directory),
            Entry::new(EntryId(2), path.join("filled"), EntryKind::Directory),
            Entry::new(EntryId(3), path.join("link"), EntryKind::DirectoryLink),
            Entry::new(EntryId(4), path.join("file"), EntryKind::File),
        ],
    );
    p.panes[0].set_tree_viewport((0, 20));
    p
}

fn request(p: &mut Presenter) -> Arc<FolderPresenceRequest> {
    let requests = RefCell::new(Vec::new());
    p.flush_folder_presence(&|command| {
        let Command::ProbeFolderPresence(request) = command else {
            panic!("probing must not enable tree mode or change selection")
        };
        requests.borrow_mut().push(request);
        true
    });
    let requests = requests.into_inner();
    assert_eq!(requests.len(), 1);
    requests.into_iter().next().unwrap()
}

fn result(request: &FolderPresenceRequest) -> Box<FolderPresenceSnapshot> {
    Box::new(FolderPresenceSnapshot {
        root: request.root.clone(),
        revision: request.revision,
        values: request
            .visible
            .iter()
            .map(|id| {
                (
                    *id,
                    if id.0 == 1 {
                        FolderPresence::Empty
                    } else {
                        FolderPresence::HasFolders
                    },
                )
            })
            .collect::<Vec<_>>()
            .into(),
    })
}

#[test]
fn flat_hints_arrive_before_click_without_changing_selection_or_flat_authority() {
    let mut p = ready();
    let selection = p.panes[0].selected.clone();
    let cursor = p.panes[0].cursor;
    let request = request(&mut p);
    assert_eq!(request.visible, [EntryId(1), EntryId(2)]);
    p.apply(Event::FolderPresence(result(&request)));
    assert_eq!(
        p.panes[0].root_presence_disclosure(&p.panes[0].entries[0]),
        5
    );
    assert_eq!(
        p.panes[0].root_presence_disclosure(&p.panes[0].entries[1]),
        3
    );
    assert_eq!(
        p.panes[0].root_presence_disclosure(&p.panes[0].entries[2]),
        0
    );
    assert!(p.panes[0].tree_view().is_none());
    assert!(p.panes[0].native_capabilities());
    assert!(Arc::ptr_eq(&selection, &p.panes[0].selected));
    assert_eq!(p.panes[0].cursor, cursor);
    p.flush_folder_presence(&|_| panic!("completed viewport values must be cached"));
    let identity = p.panes[0].row_identity(&p.panes[0].entries[1]);
    let commands = RefCell::new(Vec::new());
    p.disclose_tree(PaneId::Left, &identity, &|command| {
        commands.borrow_mut().push(command);
        true
    });
    assert!(commands.borrow().iter().any(|command| matches!(command, Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Enable { root, .. } if root.same_snapshot(&request.root)))));
}

#[test]
fn flat_presence_retries_outer_queue_and_rejects_foreign_or_superseded_receipts() {
    let mut p = ready();
    let rejected = RefCell::new(None);
    p.flush_folder_presence(&|command| {
        let Command::ProbeFolderPresence(request) = command else {
            panic!()
        };
        *rejected.borrow_mut() = Some(request);
        false
    });
    let first = request(&mut p);
    assert!(Arc::ptr_eq(rejected.borrow().as_ref().unwrap(), &first));
    let mut foreign = result(&first);
    foreign.root.index = Arc::new((*foreign.root.index).clone());
    p.apply(Event::FolderPresence(foreign));
    assert_eq!(
        p.panes[0].root_presence_disclosure(&p.panes[0].entries[0]),
        1
    );
    let offset = p.panes[0].parent_offset();
    p.panes[0].set_tree_viewport((offset + 1, offset + 2));
    let newer = request(&mut p);
    assert!(first.cancel.is_cancelled());
    p.apply(Event::FolderPresence(result(&first)));
    assert_eq!(
        p.panes[0].root_presence_disclosure(&p.panes[0].entries[0]),
        1
    );
    p.apply(Event::FolderPresence(result(&newer)));
    assert_eq!(
        p.panes[0].root_presence_disclosure(&p.panes[0].entries[1]),
        3
    );
}

#[test]
fn flat_presence_navigation_cancels_work_and_failed_probe_keeps_its_reason() {
    let mut p = ready();
    let first = request(&mut p);
    let error = TreeDiagnostic::new("Access denied while checking subfolders");
    let values = first
        .visible
        .iter()
        .map(|id| (*id, FolderPresence::Unavailable(error.clone())))
        .collect::<Vec<_>>()
        .into();
    p.apply(Event::FolderPresence(Box::new(FolderPresenceSnapshot {
        root: first.root.clone(),
        revision: first.revision,
        values,
    })));
    assert_eq!(
        p.panes[0].root_presence_disclosure(&p.panes[0].entries[0]),
        6
    );
    assert!(
        p.panes[0]
            .root_presence_error(EntryId(1))
            .contains("Access denied")
    );
    let identity = p.panes[0].row_identity(&p.panes[0].entries[0]);
    p.inspect_panel_warning(PaneId::Left, &identity, true, &|_| {
        panic!("bounded detail is already captured")
    });
    let (title, body) = p.desktop.notice.as_ref().expect("warning detail notice");
    assert_eq!(title, "Folder listing error");
    assert!(body.contains("Access denied"));
    assert!(body.contains("empty"));
    p.clear_folder_presence(PaneId::Left);
    let pending = request(&mut p);
    begin(
        &mut p,
        PaneId::Left,
        test_root().join("elsewhere").to_str().unwrap(),
    );
    assert!(pending.cancel.is_cancelled());
    p.apply(Event::FolderPresence(result(&pending)));
    assert!(p.panes[0].tree_view().is_none());
}

#[test]
fn flat_presence_request_is_bounded_to_the_viewport_not_the_whole_listing() {
    let mut p = ready();
    let path = p.panes[0].path.clone();
    let navigation = begin(&mut p, PaneId::Left, path.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        (0..600)
            .map(|id| {
                Entry::new(
                    EntryId(id),
                    path.join(format!("folder{id}")),
                    EntryKind::Directory,
                )
            })
            .collect(),
    );
    let offset = p.panes[0].parent_offset();
    p.panes[0].set_tree_viewport((offset + 20, offset + 600));
    let request = request(&mut p);
    assert_eq!(request.visible.len(), FolderPresenceRequest::MAX_VISIBLE);
    assert_eq!(request.visible[0], p.panes[0].entries[20].id);
    assert_eq!(*request.visible.last().unwrap(), p.panes[0].entries[275].id);
}

#[test]
fn red_folder_error_icon_uses_modal_details_instead_of_a_viewer() {
    #[derive(Debug)]
    struct BackendOnly;
    impl GeneratedTextSource for BackendOnly {
        fn length(&self) -> u64 {
            panic!("UI must not inspect diagnostic storage")
        }
        fn read_at(&self, _: u64, _: &mut [u8]) -> io::Result<usize> {
            panic!("UI must not read diagnostic storage")
        }
    }
    let mut p = ready();
    p.viewer_plugins.startup_disabled = Some(Arc::from([]));
    let request = request(&mut p);
    let mut snapshot = result(&request);
    snapshot.values = vec![
        (
            EntryId(1),
            FolderPresence::Unavailable(TreeDiagnostic::with_details(
                "Access denied",
                Arc::new(BackendOnly),
            )),
        ),
        (EntryId(2), FolderPresence::HasFolders),
    ]
    .into();
    p.apply(Event::FolderPresence(snapshot));
    let identity = p.panes[0].row_identity(&p.panes[0].entries[0]);
    let full_path = p.panes[0].entries[0].path.to_string_lossy().into_owned();
    let sent = RefCell::new(None);
    p.inspect_panel_warning(PaneId::Left, &identity, true, &|command| {
        *sent.borrow_mut() = Some(command);
        true
    });
    let Command::ReadDiagnosticText { request, .. } = sent.into_inner().unwrap() else {
        panic!("red error icon must request the application's modal details");
    };
    assert!(p.take_warning_viewer().is_none());
    let (title, body) = p.desktop.notice.as_ref().unwrap();
    assert_eq!(title, "Folder listing error");
    assert!(body.contains("Access denied") && body.contains(&full_path));
    let full = "Provider details without a repeated path".to_owned();
    p.apply(Event::DiagnosticTextReady {
        request,
        result: Ok(DiagnosticTextPreview {
            text: full.clone(),
            total_bytes: full.len() as u64,
            truncated: false,
        }),
    });
    let body = &p.desktop.notice.as_ref().unwrap().1;
    assert!(body.contains(&full) && body.contains(&full_path) && body.contains("Access denied"));
    assert!(p.take_warning_viewer().is_none());
}
