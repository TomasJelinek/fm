use super::*;
use std::{cell::RefCell, io};

fn fixture(send: &dyn Fn(Command) -> bool) -> (Presenter, DesktopRequestId) {
    let root = std::env::temp_dir();
    let mut p = Presenter::new(root.clone(), root.clone());
    p.preferences.loaded = true;
    p.viewer_plugins.startup_disabled = Some(Arc::from([]));
    let pane = &mut p.panes[0];
    pane.entries = vec![
        Entry::new(EntryId(1), root.join("文档.txt"), EntryKind::File),
        Entry::new(EntryId(2), root.join("folder"), EntryKind::Directory),
    ]
    .into();
    pane.index = Arc::new(DirectoryIndex::from_entries(&pane.entries));
    pane.cursor = Some(EntryId(1));
    pane.parent_cursor = false;
    pane.loading = false;
    p.request_make_file_list(PaneId::Left, send);
    let id = p.file_list.session.as_ref().unwrap().id;
    (p, id)
}

fn fed_fixture() -> Presenter {
    let root = std::env::temp_dir();
    let mut p = Presenter::new(root.clone(), root.clone());
    let id = p.create_find(PaneId::Left).unwrap();
    let s = p.finds.sessions.get_mut(&id).unwrap();
    s.results = (1..=2)
        .map(|n| {
            Arc::new(FindResult {
                id: FindResultId(n),
                entry: FindEntry {
                    path: root.join(format!("result-folder-{n}/文档.txt")),
                    kind: EntryKind::File,
                    metadata: EntryMetadata::default(),
                    identity: TransferIdentity {
                        device: 1,
                        object: [n as u8; 16],
                    },
                    revision: FindRevision([0; 6]),
                },
                duplicate_group: None,
                duplicate_group_size: 0,
                archive: None,
            })
        })
        .collect();
    p.feed_find_results(id).unwrap();
    p.preferences.loaded = true;
    p
}

#[test]
fn fed_results_capture_their_own_paths_marks_and_cursor_for_make_file_list() {
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    let root = std::env::temp_dir();
    let mut p = fed_fixture();
    p.feed_click(PaneId::Left, 1, true, false);
    p.request_make_file_list(PaneId::Left, &send);
    let session = p
        .file_list
        .session
        .as_ref()
        .expect("Make File List from results");
    let capture = session.capture.clone().unwrap();
    let FileListSource::Native {
        entries,
        selection,
        fallback,
        ..
    } = &capture.source
    else {
        panic!()
    };
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[1].path, root.join("result-folder-2/文档.txt"));
    assert_eq!(selection.len(), 1);
    assert!(selection.contains_path(&root.join("result-folder-2/文档.txt")));
    assert_eq!(*fallback, None);
    let id = session.id;
    p.feed_close(PaneId::Left);
    p.panes[0].entries = Arc::from([]);
    p.panes[0].generation += 1;
    let session = p.file_list.session.as_ref().unwrap();
    assert!(Arc::ptr_eq(session.capture.as_ref().unwrap(), &capture));
    p.generate_file_list(id, 0, String::new(), false, &send);
    let commands = commands.borrow();
    let Command::RunFileList(request) = commands.last().unwrap() else {
        panic!()
    };
    assert!(Arc::ptr_eq(&request.capture, &capture));
}

#[test]
fn fed_file_list_without_marks_falls_back_only_to_the_focused_result() {
    let mut p = fed_fixture();
    p.feed_click(PaneId::Left, 1, false, false);
    p.open_feed_file_list(PaneId::Left, &|_| true);
    let capture = p
        .file_list
        .session
        .as_ref()
        .unwrap()
        .capture
        .as_ref()
        .unwrap();
    let FileListSource::Native {
        entries,
        selection,
        fallback,
        ..
    } = &capture.source
    else {
        panic!()
    };
    assert!(selection.is_empty());
    assert_eq!(*fallback, Some(EntryId(2)));
    assert_eq!(
        entries[1].path,
        std::env::temp_dir().join("result-folder-2/文档.txt")
    );
}

fn finish_fed_file_list(p: &mut Presenter, send: &dyn Fn(Command) -> bool) {
    let id = p.file_list.session.as_ref().unwrap().id;
    let path = std::env::temp_dir().join("fed-file-list.txt");
    p.generate_file_list(id, 2, path.to_string_lossy().into_owned(), false, send);
    let generation = p.file_list.session.as_ref().unwrap().generation;
    p.apply_file_list(FileListEvent {
        id,
        generation,
        result: Ok(FileListUpdate::Saved {
            path,
            bytes: 0,
            recovery: Vec::new(),
            warning: None,
        }),
    });
    p.flush_file_list(send);
    assert!(p.file_list.session.is_none());
}

#[test]
fn completed_fed_file_list_clears_only_its_feed_marks_and_never_the_folder_selection() {
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    let mut p = fed_fixture();
    let pane = &mut p.panes[0];
    pane.entries = Arc::from([Entry::new(
        EntryId(9),
        std::env::temp_dir().join("underlying-folder-file.txt"),
        EntryKind::File,
    )]);
    pane.index = Arc::new(DirectoryIndex::from_entries(&pane.entries));
    pane.selected = Arc::new(SelectionSnapshot::from_bits(pane.index.clone(), vec![1]).unwrap());
    let selected = pane.selected.clone();
    p.feed_click(PaneId::Left, 1, true, false);
    p.request_make_file_list(PaneId::Left, &send);
    finish_fed_file_list(&mut p, &send);
    assert_eq!(p.feed(PaneId::Left).unwrap().selected_count, 0);
    assert!(Arc::ptr_eq(&selected, &p.panes[0].selected));
    assert!(
        !commands
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::Selection(_)))
    );
}

#[test]
fn completed_fed_file_list_preserves_new_marks_or_a_replacement_feed() {
    for replacement in [false, true] {
        let mut p = fed_fixture();
        p.feed_click(PaneId::Left, 1, true, false);
        p.request_make_file_list(PaneId::Left, &|_| true);
        if replacement {
            let id = *p.finds.sessions.keys().next().unwrap();
            p.feed_find_results(id).unwrap();
        }
        p.feed_click(PaneId::Left, 0, true, false);
        let selected = p.feed(PaneId::Left).unwrap().selected.clone();
        finish_fed_file_list(&mut p, &|_| true);
        assert_eq!(p.feed(PaneId::Left).unwrap().selected, selected);
    }
}

#[derive(Debug)]
struct Text;
impl GeneratedTextSource for Text {
    fn length(&self) -> u64 {
        3
    }
    fn read_at(&self, offset: u64, out: &mut [u8]) -> io::Result<usize> {
        let data = b"abc".get(offset as usize..).unwrap_or_default();
        let len = out.len().min(data.len());
        out[..len].copy_from_slice(&data[..len]);
        Ok(len)
    }
}

fn generated(p: &mut Presenter, id: DesktopRequestId) {
    let generation = p.file_list.session.as_ref().unwrap().generation;
    p.apply_file_list(FileListEvent {
        id,
        generation,
        result: Ok(FileListUpdate::Generated {
            content: Arc::new(Text),
            items: 1,
        }),
    });
}

#[test]
fn captured_view_is_reused_after_navigation_and_parent_is_never_a_fallback() {
    let commands = RefCell::new(Vec::new());
    let send = |c| {
        commands.borrow_mut().push(c);
        true
    };
    let (mut p, id) = fixture(&send);
    let captured = p
        .file_list
        .session
        .as_ref()
        .unwrap()
        .capture
        .clone()
        .unwrap();
    p.panes[0].generation += 1;
    p.panes[0].entries = Arc::from([]);
    p.generate_file_list(id, 1, String::new(), false, &send);
    let commands = commands.borrow();
    let Command::RunFileList(request) = commands.last().unwrap() else {
        panic!()
    };
    assert!(Arc::ptr_eq(&captured, &request.capture));
    let FileListSource::Native {
        entries, fallback, ..
    } = &request.capture.source
    else {
        panic!()
    };
    assert_eq!(entries.len(), 2);
    assert_eq!(*fallback, Some(EntryId(1)));
    drop(commands);
    p.close_file_list(id, &send);
    p.panes[0].parent_cursor = true;
    p.request_make_file_list(PaneId::Left, &send);
    let captured = p
        .file_list
        .session
        .as_ref()
        .unwrap()
        .capture
        .as_ref()
        .unwrap();
    let FileListSource::Native { fallback, .. } = &captured.source else {
        panic!()
    };
    assert_eq!(*fallback, None);
}

#[test]
fn the_item_count_is_singular_for_one_item() {
    let send = |_| true;
    let (mut p, id) = fixture(&send);
    let generation = p.file_list.session.as_ref().unwrap().generation;
    for (items, truncated, summary) in [
        (1, false, "1 item"),
        (2, true, "2 items · preview shortened"),
    ] {
        p.apply_file_list(FileListEvent {
            id,
            generation,
            result: Ok(FileListUpdate::Preview {
                text: "preview".into(),
                items,
                truncated,
            }),
        });
        assert_eq!(p.file_list.session.as_ref().unwrap().summary, summary);
    }
    generated(&mut p, id);
    assert_eq!(p.file_list.session.as_ref().unwrap().summary, "1 item");
}

#[test]
fn stale_previews_cannot_replace_new_draft_and_close_cancels_last_submitted_generation() {
    let commands = RefCell::new(Vec::new());
    let send = |c| {
        commands.borrow_mut().push(c);
        true
    };
    let (mut p, id) = fixture(&send);
    p.edit_file_list_template(id, "$(FileSize)".into());
    p.apply_file_list(FileListEvent {
        id,
        generation: 1,
        result: Ok(FileListUpdate::Preview {
            text: "obsolete".into(),
            items: 1,
            truncated: false,
        }),
    });
    assert!(p.file_list.session.as_ref().unwrap().preview.is_empty());
    assert!(p.close_file_list(id, &send));
    assert!(matches!(
        commands.borrow().last(),
        Some(Command::CancelFileList { generation: 1, .. })
    ));
    crate::presenter::tests::assert_late_event_is_only_retired(&mut p, |p| {
        p.apply_file_list(FileListEvent {
            id,
            generation: 2,
            result: Ok(FileListUpdate::Preview {
                text: "late".into(),
                items: 1,
                truncated: false,
            }),
        })
    });
    assert!(p.file_list.session.is_none());
}

#[test]
fn clipboard_completion_waits_for_ack_and_does_not_clear_a_new_selection() {
    let commands = RefCell::new(Vec::new());
    let send = |c| {
        commands.borrow_mut().push(c);
        true
    };
    let (mut p, id) = fixture(&send);
    p.edit_file_list_template(id, "$(FileName)$(LF)".into());
    p.generate_file_list(id, 0, String::new(), false, &send);
    generated(&mut p, id);
    p.flush_file_list(&send);
    let request = match commands.borrow().last().unwrap() {
        Command::CopyGeneratedText { request, .. } => *request,
        _ => panic!(),
    };
    assert!(p.file_list.session.is_some());
    assert_eq!(
        p.preferences.values.file_list.template_history[0],
        DEFAULT_FILE_LIST_TEMPLATE
    );
    p.panes[0].selection_revision = SelectionRevision(1);
    p.file_list_clipboard_finished(request, Ok(DesktopActionResult::ClipboardCopied));
    commands.borrow_mut().clear();
    p.flush_file_list(&send);
    assert!(p.file_list.session.is_none());
    assert_eq!(
        p.preferences.values.file_list.template_history[0],
        "$(FileName)$(LF)"
    );
    assert!(
        !commands
            .borrow()
            .iter()
            .any(|c| matches!(c, Command::Selection(_)))
    );
}

#[test]
fn failed_clipboard_keeps_draft_and_preserves_history() {
    let (mut p, id) = fixture(&|_| true);
    p.generate_file_list(id, 0, String::new(), false, &|_| true);
    generated(&mut p, id);
    p.flush_file_list(&|_| true);
    let request = p
        .file_list
        .session
        .as_ref()
        .unwrap()
        .clipboard_request
        .unwrap();
    let before = p.preferences.values.file_list.clone();
    p.file_list_clipboard_finished(
        request,
        Err(FsError::from_io(
            "clipboard",
            &std::env::temp_dir(),
            io::ErrorKind::PermissionDenied.into(),
        )),
    );
    p.flush_file_list(&|_| true);
    let s = p.file_list.session.as_ref().unwrap();
    assert!(!s.busy());
    assert!(!s.error.is_empty());
    assert_eq!(p.preferences.values.file_list, before);
}

#[test]
fn waiting_selection_never_retargets_after_another_selection_intent() {
    let (mut p, id) = fixture(&|_| true);
    p.close_file_list(id, &|_| true);
    p.panes[0].selection_revision = SelectionRevision(1);
    p.request_make_file_list(PaneId::Left, &|_| true);
    assert!(p.file_list.session.as_ref().unwrap().waiting);
    p.panes[0].selection_revision = SelectionRevision(2);
    p.panes[0].selection_resolved = SelectionRevision(2);
    p.flush_file_list(&|_| panic!("a newer selection must never be submitted"));
    let s = p.file_list.session.as_ref().unwrap();
    assert!(s.capture.is_none());
    assert!(!s.error.is_empty());
}

#[test]
fn conflict_requires_explicit_confirmation_and_native_picker_never_exports() {
    let commands = RefCell::new(Vec::new());
    let send = |c| {
        commands.borrow_mut().push(c);
        true
    };
    let (mut p, id) = fixture(&send);
    let typed = std::env::temp_dir().join("edited-list.txt");
    p.request_file_list_picker(id, &typed.to_string_lossy(), PathPickerOwner::None, &send);
    assert!(
        matches!(commands.borrow().last(), Some(Command::PickPath { input, .. })
        if input.initial_name.as_deref() == typed.file_name() && input.initial_directory.as_deref() == typed.parent())
    );
    let request = p.file_list.session.as_ref().unwrap().picker.unwrap();
    let path = std::env::temp_dir().join("文档-list.txt");
    commands.borrow_mut().clear();
    p.file_list_path_picked(request, Ok(Some(path.clone())));
    assert!(commands.borrow().is_empty());
    p.generate_file_list(id, 2, path.to_string_lossy().into_owned(), false, &send);
    let generation = p.file_list.session.as_ref().unwrap().generation;
    p.apply_file_list(FileListEvent {
        id,
        generation,
        result: Ok(FileListUpdate::Conflict {
            entry: TransferEntry {
                item: MutationItem {
                    path,
                    kind: EntryKind::File,
                    metadata: EntryMetadata {
                        size: Some(1024),
                        ..Default::default()
                    },
                },
                identity: TransferIdentity {
                    device: 1,
                    object: [0; 16],
                },
            },
            append: false,
        }),
    });
    commands.borrow_mut().clear();
    p.flush_file_list(&send);
    assert!(commands.borrow().is_empty());
    assert!(p.file_list.session.as_ref().unwrap().conflict.is_some());
    p.confirm_file_list(id, &send);
    assert!(matches!(
        commands.borrow().last(),
        Some(Command::ConfirmFileList { .. })
    ));
}

#[test]
fn completion_retries_selection_clear_backpressure_and_history_is_saved_once() {
    let (mut p, id) = fixture(&|_| true);
    p.generate_file_list(id, 1, String::new(), false, &|_| true);
    generated(&mut p, id);
    let (viewer_id, generation, launch) = p.take_generated_file_list_viewer().unwrap();
    p.generated_file_list_viewer_finished(viewer_id, generation, Ok(()));
    let commands = RefCell::new(Vec::new());
    p.flush_file_list(&|command| {
        let accepted = !matches!(command, Command::Selection(_));
        commands.borrow_mut().push(command);
        accepted
    });
    assert!(p.file_list.session.is_some());
    commands.borrow_mut().clear();
    p.flush_file_list(&|c| {
        commands.borrow_mut().push(c);
        true
    });
    assert!(p.file_list.session.is_none());
    assert!(matches!(
        commands.borrow().as_slice(),
        [Command::Selection(_)]
    ));
    assert!(launch.generated.is_some());
    assert!(p.take_generated_file_list_viewer().is_none());
}

#[test]
fn close_cancels_queued_clipboard_even_when_backend_command_queue_is_full() {
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    let (mut p, id) = fixture(&send);
    p.generate_file_list(id, 0, String::new(), false, &send);
    generated(&mut p, id);
    p.flush_file_list(&send);
    let lease = match commands.borrow().last().unwrap() {
        Command::CopyGeneratedText { cancel, .. } => cancel.clone(),
        _ => panic!(),
    };
    assert!(!lease.is_cancelled());
    assert!(!p.close_file_list(id, &|_| false));
    assert!(lease.is_cancelled());
    assert!(p.file_list.session.as_ref().unwrap().closing);
    p.flush_file_list(&|_| true);
    assert!(p.file_list.session.is_none());
}

#[test]
fn cancelling_file_list_conflict_blocks_confirmation_while_cancel_queue_is_full() {
    for append in [false, true] {
        let (mut p, id) = fixture(&|_| true);
        let path = std::env::temp_dir().join("existing-list.txt");
        p.generate_file_list(id, 2, path.to_string_lossy().into_owned(), append, &|_| {
            true
        });
        let generation = p.file_list.session.as_ref().unwrap().generation;
        p.apply_file_list(FileListEvent {
            id,
            generation,
            result: Ok(FileListUpdate::Conflict {
                entry: TransferEntry {
                    item: MutationItem {
                        path,
                        kind: EntryKind::File,
                        metadata: EntryMetadata::default(),
                    },
                    identity: TransferIdentity {
                        device: 1,
                        object: [0; 16],
                    },
                },
                append,
            }),
        });
        assert!(p.file_list.session.as_ref().unwrap().can_confirm());
        let template = p.file_list.session.as_ref().unwrap().template.clone();
        assert!(!p.close_file_list(id, &|command| {
            assert!(matches!(command, Command::CancelFileList { id: actual, generation: current } if actual == id && current == generation));
            false
        }));
        let session = p.file_list.session.as_ref().unwrap();
        assert!(session.closing);
        assert!(!session.can_confirm(), "closing must hide Replace/Append");
        assert!(
            session.conflict.is_some(),
            "retain the draft until cancellation is queued"
        );
        assert_eq!(session.template, template);
        assert_eq!(session.append, append);
        p.confirm_file_list(id, &|_| panic!("Cancel must prevent later confirmation"));
        p.flush_file_list(&|command| {
            assert!(matches!(command, Command::CancelFileList { id: actual, generation: current } if actual == id && current == generation));
            true
        });
        assert!(p.file_list.session.is_none());
    }
}

#[test]
fn viewer_failure_keeps_draft_and_cancel_discards_undelivered_viewer() {
    let (mut p, id) = fixture(&|_| true);
    p.generate_file_list(id, 1, String::new(), false, &|_| true);
    generated(&mut p, id);
    p.flush_file_list(&|_| true);
    assert!(p.file_list.session.is_some());
    let (viewer_id, generation, _) = p.take_generated_file_list_viewer().unwrap();
    p.generated_file_list_viewer_finished(
        viewer_id,
        generation,
        Err("Text viewer is unavailable".into()),
    );
    p.flush_file_list(&|_| true);
    assert!(!p.file_list.session.as_ref().unwrap().busy());
    assert_eq!(p.preferences.values.file_list.destination, 0);
    p.generate_file_list(id, 1, String::new(), false, &|_| true);
    generated(&mut p, id);
    p.close_file_list(id, &|_| true);
    assert!(p.take_generated_file_list_viewer().is_none());
}

#[test]
fn generated_viewer_waits_for_startup_plugin_choices_instead_of_default_enabling() {
    let (mut p, id) = fixture(&|_| true);
    p.viewer_plugins.startup_disabled = None;
    let before = p.preferences.values.file_list.clone();
    p.generate_file_list(id, 1, String::new(), false, &|_| true);
    generated(&mut p, id);
    assert!(p.take_generated_file_list_viewer().is_none());
    let session = p.file_list.session.as_ref().unwrap();
    assert!(!session.running);
    assert!(!session.viewer_pending);
    assert!(session.error.contains("loading"));
    assert_eq!(p.preferences.values.file_list, before);

    p.viewer_plugins.startup_disabled = Some(Arc::from(["viewer.basic".into()]));
    p.generate_file_list(id, 1, String::new(), false, &|_| true);
    generated(&mut p, id);
    let (_, _, launch) = p.take_generated_file_list_viewer().unwrap();
    assert_eq!(&*launch.disabled_plugins, &["viewer.basic"]);
}

#[test]
fn late_viewer_result_never_completes_another_file_list_session() {
    let (mut p, id) = fixture(&|_| true);
    p.generate_file_list(id, 1, String::new(), false, &|_| true);
    generated(&mut p, id);
    let (_, old_generation, _) = p.take_generated_file_list_viewer().unwrap();
    p.close_file_list(id, &|_| true);
    p.request_make_file_list(PaneId::Left, &|_| true);
    let new_id = p.file_list.session.as_ref().unwrap().id;
    p.generate_file_list(new_id, 1, String::new(), false, &|_| true);
    generated(&mut p, new_id);
    let before = p.preferences.values.file_list.clone();
    p.generated_file_list_viewer_finished(id, old_generation, Ok(()));
    p.flush_file_list(&|_| true);
    let current = p.file_list.session.as_ref().unwrap();
    assert_eq!(current.id, new_id);
    assert!(current.viewer_pending);
    assert!(current.running);
    assert!(!current.completed);
    assert_eq!(p.preferences.values.file_list, before);
}

#[test]
fn duplicate_viewer_result_never_completes_a_retried_generation() {
    let (mut p, id) = fixture(&|_| true);
    p.generate_file_list(id, 1, String::new(), false, &|_| true);
    generated(&mut p, id);
    let (_, generation, _) = p.take_generated_file_list_viewer().unwrap();
    p.generated_file_list_viewer_finished(id, generation, Err("viewer unavailable".into()));
    p.generate_file_list(id, 1, String::new(), false, &|_| true);
    generated(&mut p, id);
    p.generated_file_list_viewer_finished(id, generation, Ok(()));
    let current = p.file_list.session.as_ref().unwrap();
    assert!(current.generation > generation);
    assert!(current.viewer_pending);
    assert!(!current.completed);
}

#[test]
fn failed_archive_selection_never_captures_the_previous_selection() {
    let (mut p, id) = fixture(&|_| true);
    p.close_file_list(id, &|_| true);
    let root = std::env::temp_dir();
    let mut archive = archives::ArchivePane::unavailable(
        ArchiveAddress {
            nested: Vec::new(),
            provider: "archive".into(),
            format: Some(ArchiveFormat::Zip),
            container: root.join("data.zip"),
            source_version: vec![],
            components: vec![],
        },
        root.clone(),
    );
    archive.selection_pending = true;
    archive.selection_revision = SelectionRevision(1);
    p.panes[0].archive = Some(archive);
    p.request_make_file_list(PaneId::Left, &|_| panic!("wait for exact selection"));
    assert!(p.file_list.session.as_ref().unwrap().waiting);
    p.apply(Event::ArchiveSelection {
        pane: PaneId::Left,
        navigation: NavigationId(p.panes[0].generation),
        revision: SelectionRevision(1),
        view: p.panes[0].display_index().clone(),
        result: Err(FsError::from_io(
            "selection",
            &root,
            io::ErrorKind::WouldBlock.into(),
        )),
    });
    p.flush_file_list(&|_| panic!("failed selection must never generate a list"));
    let s = p.file_list.session.as_ref().unwrap();
    assert!(!s.waiting);
    assert!(s.capture.is_none());
    assert!(s.error.contains("selection failed"));
}
