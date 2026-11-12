use super::*;

fn loaded() -> Presenter {
    let (mut p, root) = super::tree::fixture();
    p.preferences_loaded(Ok(UserPreferences::default()));
    let publication = super::tree::publication(root, 1, true, true);
    let mut rows = publication.view.rows.to_vec();
    rows[1].entry.kind = EntryKind::File;
    rows[1].entry.display_name = "misleading caption.txt".into();
    rows[1].entry.path = std::env::current_dir()
        .unwrap()
        .join("not-a-native-authority");
    rows[1].disclosure = TreeDisclosure::None;
    rows[1].navigable = false;
    let mut root_file = rows[1].clone();
    root_file.entry = publication.view.root.entries[1].clone();
    root_file.id = TreeRowId(31);
    root_file.entry.id = EntryId(31);
    root_file.parent = None;
    root_file.depth = 0;
    root_file.original_root_entry = Some(EntryId(11));
    rows.push(root_file);
    let view = Arc::new(
        TreeProjection::new(
            publication.view.root.clone(),
            publication.view.stamp,
            publication.view.layout_revision,
            rows,
        )
        .unwrap(),
    );
    let selection = Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![1]).unwrap());
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(
            TreePublication::new(view, SelectionRevision(0), selection).unwrap(),
        ))),
        &|_| true,
    );
    p.panes[0].cursor = Some(EntryId(11));
    p.panes[0].parent_cursor = false;
    p
}

fn capture(p: &mut Presenter) -> TreeEditorRequest {
    let commands = RefCell::new(Vec::new());
    p.edit_file(PaneId::Left, &|command| {
        commands.borrow_mut().push(command);
        true
    });
    commands
        .into_inner()
        .into_iter()
        .find_map(|command| match command {
            Command::Tree(command) => match *command {
                TreeCommand::Edit(request) => Some(request),
                _ => None,
            },
            _ => None,
        })
        .expect("F4 must capture the focused tree occurrence")
}

#[derive(Debug)]
struct Source {
    stamp: TreeViewStamp,
    row: TreeRowId,
}
impl CapturedEditorFile for Source {
    fn stamp(&self) -> TreeViewStamp {
        self.stamp
    }
    fn row(&self) -> TreeRowId {
        self.row
    }
    fn resolve(&self, _: &Cancellation) -> Result<PathBuf, FsError> {
        panic!("UI must never resolve a native editor path")
    }
}
fn receipt(request: &TreeEditorRequest) -> Event {
    Event::Tree(Box::new(TreeEvent::EditorCaptured {
        stamp: request.view.stamp,
        request: request.request,
        result: Ok(Arc::new(Source {
            stamp: request.view.stamp,
            row: request.row,
        })),
    }))
}

#[test]
fn tree_editor_captures_root_or_descendant_cursor_independent_of_selected_directory() {
    for row in [11, 31] {
        let mut p = loaded();
        p.panes[0].cursor = Some(EntryId(row));
        let request = capture(&mut p);
        assert_eq!(request.row, TreeRowId(row));
        assert_eq!(request.view.stamp.pane, PaneId::Left);
        assert_eq!(request.view.rows[0].entry.kind, EntryKind::Directory);
        p.active = PaneId::Right;
        let commands = RefCell::new(Vec::new());
        p.apply_with_sender(receipt(&request), &|command| {
            commands.borrow_mut().push(command);
            true
        });
        let commands = commands.borrow();
        let Command::EditCapturedFile { source, .. } = &commands[0] else {
            panic!("captured editor launch");
        };
        assert_eq!(source.stamp().pane, PaneId::Left);
        assert_eq!(source.row(), TreeRowId(row));
        assert!(
            !commands
                .iter()
                .any(|command| matches!(command, Command::EditFile { .. }))
        );
    }
}

#[test]
fn tree_editor_rejects_parent_directory_loading_and_uncommitted_view() {
    for blocked in 0..4 {
        let mut p = loaded();
        match blocked {
            0 => p.panes[0].parent_cursor = true,
            1 => p.panes[0].cursor = Some(EntryId(7)),
            2 => p.panes[0].loading = true,
            _ => p.panes[0].tree.commit_pending = true,
        }
        p.edit_file(PaneId::Left, &|_| panic!("blocked tree editor request"));
    }
}

#[test]
fn tree_editor_navigation_and_shutdown_cancel_late_capture_without_launch() {
    for close in [false, true] {
        let mut p = loaded();
        let request = capture(&mut p);
        if close {
            p.request_operation_shutdown(&|_| true);
        } else {
            p.clear_tree(PaneId::Left, None);
        }
        assert!(request.cancel.is_cancelled());
        p.apply_with_sender(receipt(&request), &|command| {
            assert!(!matches!(command, Command::EditCapturedFile { .. }));
            true
        });
        assert!(p.editor.tree.is_none());
    }
}

#[test]
fn tree_editor_retries_both_queues_without_retargeting_cursor_or_settings() {
    let mut p = loaded();
    let expected_editor = p.preferences.values.editor.clone();
    p.edit_file(PaneId::Left, &|_| false);
    assert!(p.editor.tree.is_some());
    p.panes[0].cursor = Some(EntryId(31));
    p.preferences.values.editor.executable = Some(std::env::current_exe().unwrap());
    p.preferences.values.editor.arguments = vec!["--different".into(), "{file}".into()];
    let commands = RefCell::new(Vec::new());
    p.flush_tree_editor(&|command| {
        commands.borrow_mut().push(command);
        true
    });
    let Command::Tree(command) = commands.into_inner().remove(0) else {
        panic!("tree capture");
    };
    let TreeCommand::Edit(request) = *command else {
        panic!("editor capture");
    };
    assert_eq!(request.row, TreeRowId(11));
    p.apply_with_sender(receipt(&request), &|_| false);
    let commands = RefCell::new(Vec::new());
    p.flush_tree_editor(&|command| {
        commands.borrow_mut().push(command);
        true
    });
    assert!(
        matches!(&commands.borrow()[0], Command::EditCapturedFile { source, editor, .. }
        if source.row() == TreeRowId(11) && editor == &expected_editor)
    );
    p.flush_tree_editor(&|_| panic!("accepted launch must not repeat"));
}

#[test]
fn tree_editor_rejects_mismatched_capture_identity_and_duplicate_receipt() {
    let mut p = loaded();
    let request = capture(&mut p);
    let bad = Event::Tree(Box::new(TreeEvent::EditorCaptured {
        stamp: request.view.stamp,
        request: request.request,
        result: Ok(Arc::new(Source {
            stamp: request.view.stamp,
            row: TreeRowId(31),
        })),
    }));
    p.apply_with_sender(bad, &|_| panic!("mismatched occurrence cannot launch"));
    assert!(request.cancel.is_cancelled());
    p.apply_with_sender(receipt(&request), &|_| panic!("late receipt cannot launch"));
    assert!(p.editor.tree.is_none());
}

#[test]
fn tree_editor_reprojection_rejects_an_old_capture_with_the_same_session() {
    let mut p = loaded();
    let request = capture(&mut p);
    let old = p.panes[0].tree.publication.as_ref().unwrap();
    let mut stamp = old.view.stamp;
    stamp.revision = TreeRevision(stamp.revision.0 + 1);
    let view = Arc::new(
        TreeProjection::new(
            old.view.root.clone(),
            stamp,
            TreeLayoutRevision(old.view.layout_revision.0 + 1),
            old.view.rows.to_vec(),
        )
        .unwrap(),
    );
    let selection = Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![0]).unwrap());
    let publication = TreePublication::new(view, old.selection_revision, selection).unwrap();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication))),
        &|command| {
            assert!(!matches!(command, Command::EditCapturedFile { .. }));
            true
        },
    );
    p.apply_with_sender(receipt(&request), &|command| {
        assert!(!matches!(command, Command::EditCapturedFile { .. }));
        true
    });
    assert!(request.cancel.is_cancelled());
    assert!(p.editor.tree.is_none());
}
