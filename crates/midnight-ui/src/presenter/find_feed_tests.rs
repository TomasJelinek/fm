//! Feed to panel and Go to file of archive members, without a window.
use super::*;
use crate::presenter::find::FindForm;
use std::cell::RefCell;

fn entry(path: PathBuf, kind: EntryKind, size: Option<u64>) -> FindEntry {
    FindEntry {
        path,
        kind,
        metadata: EntryMetadata {
            size,
            ..EntryMetadata::default()
        },
        identity: TransferIdentity {
            device: 1,
            object: [3; 16],
        },
        revision: FindRevision([0; 6]),
    }
}

fn result(id: u64, entry: FindEntry, archive: Option<Arc<FindArchiveMember>>) -> Arc<FindResult> {
    Arc::new(FindResult {
        id: FindResultId(id),
        entry,
        duplicate_group: None,
        duplicate_group_size: 0,
        archive,
    })
}

/// A finished Find of the left panel with two files, a folder and an archive member.
fn searched() -> (Presenter, FindSessionId, PathBuf) {
    let root = std::env::temp_dir().join("feed-root");
    let mut p = Presenter::new(root.clone(), std::env::temp_dir().join("feed-target"));
    let id = p.create_find(PaneId::Left).unwrap();
    assert!(p.start_find(id, FindForm::default(), &|_| true));
    let member = Arc::new(FindArchiveMember {
        folder: ArchiveAddress {
            provider: "zip".into(),
            format: Some(ArchiveFormat::Zip),
            container: root.join("bundle.zip"),
            source_version: vec![1],
            components: vec![b"docs".to_vec()],
        },
        name: b"notes.md".to_vec(),
        container: entry(root.join("bundle.zip"), EntryKind::File, Some(9)),
    });
    let batch: Arc<[Arc<FindResult>]> = vec![
        result(
            1,
            entry(root.join("a").join("one.txt"), EntryKind::File, Some(10)),
            None,
        ),
        result(2, entry(root.join("b"), EntryKind::Directory, None), None),
        result(
            3,
            entry(root.join("b").join("two.log"), EntryKind::File, Some(20)),
            None,
        ),
        result(
            4,
            entry(
                root.join("bundle.zip").join("docs").join("notes.md"),
                EntryKind::File,
                Some(5),
            ),
            Some(member),
        ),
    ]
    .into();
    finish(&mut p, id, batch);
    (p, id, root)
}

/// The Find `id` reports `results` and completes.
fn finish(p: &mut Presenter, id: FindSessionId, results: Arc<[Arc<FindResult>]>) {
    for update in [
        FindUpdate::Batch(results),
        FindUpdate::Finished(FindSummary {
            completion: FindCompletion::Completed,
            progress: FindProgress::default(),
            reached_budgets: vec![],
            diagnostics: vec![],
        }),
    ] {
        p.apply(Event::Find(Box::new(FindEvent {
            session: id,
            generation: FindGeneration(1),
            update,
        })));
    }
}

/// The rows of `fed_with_folders`, in their order, with the files' sizes.
const ROWS: [(&str, Option<u64>); 5] = [
    ("sub", None),
    ("a.txt", Some(1)),
    ("other", None),
    ("b.txt", Some(2)),
    ("c.txt", Some(4)),
];

/// The left panel shows two folders and three files (`ROWS`), `marks` marked.
fn fed_with_folders(marks: &[&str]) -> Presenter {
    let root = std::env::temp_dir().join("feed-bulk");
    let mut p = Presenter::new(root.clone(), std::env::temp_dir().join("feed-target"));
    let id = p.create_find(PaneId::Left).unwrap();
    assert!(p.start_find(id, FindForm::default(), &|_| true));
    let results = ROWS
        .iter()
        .zip(1..)
        .map(|((name, size), number)| {
            let kind = if size.is_some() {
                EntryKind::File
            } else {
                EntryKind::Directory
            };
            result(number, entry(root.join(name), kind, *size), None)
        })
        .collect();
    finish(&mut p, id, results);
    p.feed_find_results(id).unwrap();
    for name in marks {
        let row = ROWS.iter().position(|(row, _)| row == name).unwrap();
        p.feed_click(PaneId::Left, row, true, false);
    }
    p
}

/// The names the list marks, in row order.
fn marked(p: &Presenter) -> Vec<&'static str> {
    let feed = p.feed(PaneId::Left).unwrap();
    ROWS.iter()
        .zip(&feed.selected)
        .filter_map(|((name, _), selected)| selected.then_some(*name))
        .collect()
}

/// A sender that keeps every command.
fn record(commands: &RefCell<Vec<Command>>, command: Command) -> bool {
    commands.borrow_mut().push(command);
    true
}

/// `searched()` with the left panel's own folder listed, the cursor on one of
/// its rows, and an enabled custom command: without a list of results over
/// that folder, each folder action has something to act on.
fn listed() -> (Presenter, FindSessionId, PathBuf) {
    let (mut p, id, root) = searched();
    let sent = RefCell::new(Vec::new());
    assert!(p.navigate(PaneId::Left, root.clone(), None, &|command| {
        record(&sent, command)
    }));
    let navigation = sent
        .into_inner()
        .into_iter()
        .find_map(|command| match command {
            Command::Navigate { navigation, .. } | Command::Refresh { navigation, .. } => {
                Some(navigation)
            }
            _ => None,
        })
        .expect("the folder is read");
    let entries: Arc<[Entry]> = [
        Entry::new(EntryId(1), root.join("hidden.txt"), EntryKind::File),
        Entry::new(EntryId(2), root.join("hidden"), EntryKind::Directory),
    ]
    .into();
    p.apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation,
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
    });
    let first = p.panes[0].parent_offset();
    p.focus_display_row(PaneId::Left, first);
    p.preferences.loaded = true;
    p.preferences.values.user_commands = Arc::new(vec![UserCommand {
        id: UserCommandId::new("test-command").unwrap(),
        name: "Test command".into(),
        enabled: true,
        toolbar_label: None,
        icon: None,
        working_directory: "${active.path}".into(),
        launch: UserCommandLaunch::Single,
        invocation: UserCommandInvocation::Program {
            executable: root.join("fixture-tool"),
            arguments: vec![],
        },
    }]);
    (p, id, root)
}

/// Runs `act` and checks it sent nothing and explained that `action` is not
/// available in search results.
fn refused(
    p: &mut Presenter,
    action: &str,
    act: impl FnOnce(&mut Presenter, &dyn Fn(Command) -> bool),
) {
    p.desktop.notice = None;
    let commands = RefCell::new(Vec::new());
    act(p, &|command| record(&commands, command));
    assert!(
        commands.borrow().is_empty(),
        "{action} sent commands for the folder under the list"
    );
    let (title, body) = p
        .desktop
        .notice
        .clone()
        .unwrap_or_else(|| panic!("{action} says why it is not available"));
    assert_eq!(title, REFUSED);
    assert_eq!(body, refusal(action));
}

#[test]
fn folder_actions_never_reach_the_folder_under_the_results() {
    // Without the list, the toolbar's New file and a custom command act on the folder.
    let (mut p, _, _) = listed();
    let commands = RefCell::new(Vec::new());
    let send = |command| record(&commands, command);
    p.start_operation(PaneId::Left, MutationKind::CreateFile, &send);
    p.request_user_command(PaneId::Left, "test-command", false, &send);
    assert!(
        commands
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::PrepareOperation(_)))
    );
    assert!(
        commands
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::ExecuteUserCommand(_)))
    );
    assert!(p.focused_name(PaneId::Left).is_some());

    // Under the list, the toolbar, menus, command line and custom commands refuse.
    let (mut p, id, _) = listed();
    p.feed_find_results(id).unwrap();
    let left = PaneId::Left;
    refused(&mut p, "New file", |p, send| {
        p.start_operation(left, MutationKind::CreateFile, send);
    });
    refused(&mut p, "Rename", |p, send| {
        p.start_operation(left, MutationKind::Rename, send);
    });
    refused(&mut p, "Create folder", |p, send| {
        p.start_operation(left, MutationKind::CreateDirectory, send);
    });
    refused(&mut p, "Attributes, times and case", |p, send| {
        p.open_metadata(left, false, send);
    });
    refused(&mut p, "Folder sizes", |p, send| p.folder_sizes(left, send));
    refused(&mut p, "Checksums", |p, send| {
        p.request_checksums(left, send)
    });
    assert!(p.checksums.sessions.is_empty(), "no Checksums dialog opens");
    refused(&mut p, "Make file list", |p, send| {
        p.request_make_file_list(left, send);
    });
    assert!(p.file_list.session.is_none());
    refused(&mut p, "A custom command", |p, send| {
        p.request_user_command(left, "test-command", false, send);
    });
    refused(&mut p, "Compare directories", |p, _| p.open_comparison());
    assert!(!p.comparison.open);
    refused(&mut p, "Paste", |p, send| p.paste_files(left, send));
    assert_eq!(
        p.file_compare_launch(left).err(),
        Some(refusal("Compare files")),
        "both compared panels' focus would come from the folder"
    );
    assert_eq!(
        p.focused_name(left),
        None,
        "a result's name would name a file of the folder under the list"
    );
    let right = PaneId::Right;
    assert!(
        p.file_compare_launch(right)
            .is_err_and(|error| error == refusal("Compare files")),
        "the other panel compares against the left one"
    );
}

#[test]
fn a_mask_dialog_opened_before_the_results_were_fed_leaves_the_folder_alone() {
    let (mut p, id, _) = listed();
    p.open_mask_selection(PaneId::Left, true);
    // The Find window is a window of its own: it can feed the panel meanwhile.
    p.feed_find_results(id).unwrap();
    refused(&mut p, "Select by mask", |p, send| {
        p.submit_mask_selection("*.txt", false, send);
    });
    assert!(!p.mask_dialog.open, "the dialog closes");
    assert_eq!(
        p.preferences.values.mask_selection,
        MaskSelectionPreferences::default(),
        "and remembers nothing"
    );
}

#[test]
fn properties_delete_and_the_full_path_under_the_results_use_the_results() {
    let (mut p, id, root) = listed();
    p.feed_find_results(id).unwrap();
    let result = root.join("a").join("one.txt");
    assert_eq!(p.focused_full_path(PaneId::Left), Some(result.clone()));
    // The toolbar's and F9 menu's Properties: the focused result.
    let commands = RefCell::new(Vec::new());
    p.properties(PaneId::Left, &|command| record(&commands, command));
    let Some(Command::ShowSelectionProperties {
        selection,
        fallback,
        ..
    }) = commands.borrow_mut().pop()
    else {
        panic!("Properties of the focused result");
    };
    assert!(selection.is_empty());
    assert_eq!(fallback, Some(result.clone()));
    // The shared clipboard and Copy/Move entry points take the result too.
    p.copy_files(PaneId::Left, &|command| record(&commands, command));
    let Some(Command::CopyFiles { files, .. }) = commands.borrow_mut().pop() else {
        panic!("the focused result is copied");
    };
    assert_eq!(files.fallback, Some(result.clone()));
    p.request_move_dialog(PaneId::Left, &|command| record(&commands, command));
    let draft = p.copies.draft.take().expect("the Copy/Move dialog opens");
    assert_eq!(draft.source, "Search results");
    assert_eq!(draft.intent, TransferIntent::Move);
    // A delete through the shared entry point deletes the result, as F8 does.
    p.start_operation(
        PaneId::Left,
        MutationKind::Delete(DeleteMode::Trash),
        &|command| record(&commands, command),
    );
    let prepared = commands
        .into_inner()
        .into_iter()
        .find_map(|command| match command {
            Command::PrepareCapturedDelete(request) => Some(request),
            _ => None,
        })
        .expect("the results' captured delete");
    assert_eq!(prepared.sources.root_count(), 1);
    let feed = p.feed(PaneId::Left).unwrap();
    assert_eq!(
        feed.deleting,
        Some((prepared.id, vec![result])),
        "the focused result, not the folder's focused row"
    );
}

#[test]
fn nothing_is_dropped_into_the_folder_under_the_results() {
    let rect = DropRect {
        x: 0,
        y: 0,
        width: 100,
        height: 100,
    };
    let published = |p: &Presenter| -> Vec<PaneId> {
        let snapshot = p.drag_drop.targets.lock().unwrap();
        snapshot.targets.iter().map(|target| target.pane).collect()
    };
    let (mut p, id, _) = listed();
    p.attach_drag_host(NativeDragWindow::Windows { hwnd: 1 }, &|_| true);
    p.publish_drag_targets([Some(rect), None]);
    assert_eq!(
        published(&p),
        [PaneId::Left],
        "the listed folder takes drops"
    );
    assert!(p.preview_internal_drag_target(PaneId::Left, None).is_some());

    p.feed_find_results(id).unwrap();
    p.publish_drag_targets([Some(rect), None]);
    assert!(published(&p).is_empty(), "no target under the results");
    assert!(p.preview_internal_drag_target(PaneId::Left, None).is_none());
    // A drop admitted against a target published before the list opened.
    let commands = RefCell::new(Vec::new());
    p.apply_with_sender(
        Event::DragDrop(DragDropEvent::DropAccepted {
            drag: None,
            intent: TransferIntent::Copy,
            verification: None,
            token: DropToken(3),
            pane: PaneId::Left,
            item_count: 1,
            destination: Arc::new(p.panes[0].path.clone()),
        }),
        &|command| record(&commands, command),
    );
    assert!(p.copies.progress.is_empty(), "nothing is copied there");
    assert!(
        commands
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::RejectDrop(DropToken(3))))
    );
}

#[test]
fn feeding_shows_native_results_in_the_find_panel_and_leaving_restores_the_folder() {
    let (mut p, id, root) = searched();
    p.active = PaneId::Right;
    assert_eq!(p.feed_find_results(id), Some(PaneId::Left));
    assert_eq!(p.active, PaneId::Left, "the fed panel becomes active");
    let feed = p.feed(PaneId::Left).expect("the results are shown");
    let paths: Vec<_> = feed
        .entries
        .iter()
        .map(|entry| entry.path.clone())
        .collect();
    assert_eq!(
        paths,
        [
            root.join("a").join("one.txt"),
            root.join("b"),
            root.join("b").join("two.log")
        ]
    );
    assert_eq!(feed.omitted, 1, "the archive member is not a native target");
    assert!(feed.title.starts_with("Search results (3)"));
    assert!(p.feed(PaneId::Right).is_none());
    p.feed_action(PaneId::Left, FeedAction::Leave, &|_| true);
    assert!(p.feed(PaneId::Left).is_none());
    assert_eq!(
        p.panes[0].path, root,
        "the folder under the list is unchanged"
    );
}

#[test]
fn a_running_search_is_not_fed_and_navigating_the_panel_leaves_the_list() {
    let (mut p, id, _) = searched();
    p.start_find(id, FindForm::default(), &|_| true);
    assert_eq!(p.feed_find_results(id), None);
    assert!(p.finds.sessions[&id].error.contains("Wait for the search"));
    let (mut p, id, _) = searched();
    p.feed_find_results(id).unwrap();
    p.panes[0].path = std::env::temp_dir().join("elsewhere");
    assert!(
        p.feed(PaneId::Left).is_none(),
        "another location hides the list"
    );
    p.flush_feeds();
    assert!(p.finds.feeds[0].is_none(), "and forgets it");
}

#[test]
fn marks_cursor_and_totals_follow_keys_and_clicks() {
    let (mut p, id, _) = searched();
    p.feed_find_results(id).unwrap();
    let left = PaneId::Left;
    let act = |p: &mut Presenter, action| p.feed_action(left, action, &|_| true);
    act(&mut p, FeedAction::Toggle { advance: true });
    let feed = p.feed(left).unwrap();
    assert_eq!(
        (feed.cursor, feed.selected_count, feed.selected_bytes),
        (1, 1, 10)
    );
    act(&mut p, FeedAction::Move(FeedMove::Last));
    act(&mut p, FeedAction::Toggle { advance: false });
    let feed = p.feed(left).unwrap();
    assert_eq!(
        (feed.cursor, feed.selected_count, feed.selected_bytes),
        (2, 2, 30)
    );
    act(&mut p, FeedAction::Move(FeedMove::PreviousSelected));
    assert_eq!(p.feed(left).unwrap().cursor, 0);
    // The rows are a file, a folder and a file.
    act(&mut p, FeedAction::Bulk(SelectionOperation::InvertAll));
    let feed = p.feed(left).unwrap();
    assert_eq!(
        feed.selected,
        [false, true, false],
        "invert files and folders"
    );
    assert_eq!((feed.selected_count, feed.selected_bytes), (1, 0));
    act(&mut p, FeedAction::Bulk(SelectionOperation::FilesOnly));
    let feed = p.feed(left).unwrap();
    assert_eq!(feed.selected, [true, false, true], "the files only");
    assert_eq!((feed.selected_count, feed.selected_bytes), (2, 30));
    act(&mut p, FeedAction::Bulk(SelectionOperation::Clear));
    p.feed_click(left, 0, false, false);
    p.feed_click(left, 2, false, true);
    let feed = p.feed(left).unwrap();
    assert_eq!(
        (feed.selected_count, feed.cursor),
        (3, 2),
        "Shift+click marks the range"
    );
    p.feed_click(left, 1, true, false);
    assert_eq!(
        p.feed(left).unwrap().selected,
        [true, false, true],
        "Ctrl+click toggles"
    );
}

#[test]
fn bulk_keys_mark_folders_and_files_of_the_results_as_in_a_panel() {
    // Each key from a marked folder and a marked file: (key, marks after it,
    // their count and file bytes). Rows: sub/, a.txt 1 B, other/, b.txt 2 B, c.txt 4 B.
    let cases: [(SelectionOperation, &[&str], usize, u64); 6] = [
        // Ctrl+Shift+A: every row.
        (
            SelectionOperation::All,
            &["sub", "a.txt", "other", "b.txt", "c.txt"],
            5,
            7,
        ),
        // Ctrl+A: the files replace the marks; the marked folder is unmarked.
        (
            SelectionOperation::FilesOnly,
            &["a.txt", "b.txt", "c.txt"],
            3,
            7,
        ),
        // Ctrl+D: nothing.
        (SelectionOperation::Clear, &[], 0, 0),
        // Clear files only: the folder marks stay.
        (SelectionOperation::ClearFilesOnly, &["sub"], 1, 0),
        // Ctrl+I and keypad *: the files invert; folder marks stay as they are.
        (SelectionOperation::Invert, &["sub", "b.txt", "c.txt"], 3, 6),
        // Ctrl+Shift+I: files and folders invert.
        (
            SelectionOperation::InvertAll,
            &["other", "b.txt", "c.txt"],
            3,
            6,
        ),
    ];
    for (operation, expected, count, bytes) in cases {
        let mut p = fed_with_folders(&["sub", "a.txt"]);
        let commands = RefCell::new(Vec::new());
        p.feed_action(PaneId::Left, FeedAction::Bulk(operation), &|command| {
            record(&commands, command)
        });
        assert_eq!(marked(&p), expected, "{operation:?}");
        let feed = p.feed(PaneId::Left).unwrap();
        assert_eq!(
            (feed.selected_count, feed.selected_bytes),
            (count, bytes),
            "{operation:?} totals"
        );
        assert!(
            commands.borrow().is_empty(),
            "{operation:?} reached the folder under the list"
        );
        assert_eq!(p.panes[0].selected_count(), 0, "{operation:?}");
    }
    // A second Ctrl+I restores the files' marks; folder marks never changed.
    let mut p = fed_with_folders(&["sub", "a.txt"]);
    for _ in 0..2 {
        p.feed_action(
            PaneId::Left,
            FeedAction::Bulk(SelectionOperation::Invert),
            &|_| true,
        );
    }
    assert_eq!(marked(&p), ["sub", "a.txt"]);
}

#[test]
fn copy_and_move_open_the_ordinary_dialog_with_the_marked_real_paths() {
    let (mut p, id, root) = searched();
    p.feed_find_results(id).unwrap();
    p.feed_click(PaneId::Left, 0, true, false);
    p.feed_click(PaneId::Left, 2, true, false);
    p.feed_action(
        PaneId::Left,
        FeedAction::Transfer(TransferIntent::Move),
        &|_| true,
    );
    let draft = p.copies.draft.as_ref().expect("the Copy/Move dialog opens");
    assert_eq!(draft.intent, TransferIntent::Move);
    assert_eq!(draft.destination, p.panes[1].path);
    let crate::presenter::copy_dialog::CopyDraftInput::Native(files) = &draft.input else {
        panic!("path-based native input");
    };
    assert_eq!(files.selection.len(), 2);
    assert!(
        files
            .selection
            .contains_path(&root.join("a").join("one.txt"))
    );
    assert!(
        files
            .selection
            .contains_path(&root.join("b").join("two.log"))
    );
    assert!(!files.selection.contains_path(&root.join("b")));
}

#[test]
fn a_feed_pages_by_the_rows_its_hidden_list_reported() {
    let (mut p, id, _) = searched();
    // The list exists while hidden: it reports its page before anything is fed.
    p.feed_page_rows(PaneId::Left, 7);
    p.feed_find_results(id).unwrap();
    assert_eq!(p.feed(PaneId::Left).unwrap().page, 7);
    p.feed_page_rows(PaneId::Left, 0);
    assert_eq!(p.feed(PaneId::Left).unwrap().page, 1, "at least one row");
}

#[test]
fn properties_of_marked_results_stop_their_folder_count_when_replaced() {
    let (mut p, id, root) = searched();
    p.feed_find_results(id).unwrap();
    p.feed_click(PaneId::Left, 1, true, false);
    let commands = RefCell::new(Vec::new());
    let mut cancels = Vec::new();
    for _ in 0..2 {
        p.feed_action(PaneId::Left, FeedAction::Properties, &|command| {
            record(&commands, command)
        });
        let Some(Command::ShowSelectionProperties {
            selection, cancel, ..
        }) = commands.borrow_mut().pop()
        else {
            panic!("Properties of the marked folder");
        };
        assert!(selection.contains_path(&root.join("b")));
        cancels.push(cancel);
    }
    // The summary is tracked like one for panel rows: a newer one stops the
    // earlier folder count, and its own count keeps running.
    assert!(cancels[0].is_cancelled());
    assert!(!cancels[1].is_cancelled());
}

#[test]
fn delete_enters_the_captured_delete_flow_and_a_completed_delete_drops_its_rows() {
    let (mut p, id, root) = searched();
    p.feed_find_results(id).unwrap();
    // The folder and a file inside it: deleting the folder covers the file.
    p.feed_click(PaneId::Left, 1, true, false);
    p.feed_click(PaneId::Left, 2, true, false);
    let commands = RefCell::new(Vec::new());
    p.feed_action(
        PaneId::Left,
        FeedAction::Delete(DeleteMode::Trash),
        &|command| record(&commands, command),
    );
    let prepared = commands
        .into_inner()
        .into_iter()
        .find_map(|command| match command {
            Command::PrepareCapturedDelete(request) => Some(request),
            _ => None,
        })
        .expect("the ordinary captured delete");
    assert_eq!(prepared.sources.root_count(), 1);
    assert_eq!(prepared.sources.mode(), DeleteMode::Trash);
    let id = prepared.id;
    assert!(
        p.operations.unconfirmed.contains(&id),
        "it waits for confirmation"
    );
    let progress = p
        .operations
        .progress
        .iter_mut()
        .find(|progress| progress.id == id)
        .unwrap();
    Arc::make_mut(progress).state = OperationState::Completed;
    p.flush_feeds();
    let feed = p.feed(PaneId::Left).unwrap();
    let paths: Vec<_> = feed
        .entries
        .iter()
        .map(|entry| entry.path.clone())
        .collect();
    assert_eq!(paths, [root.join("a").join("one.txt")]);
    assert_eq!(feed.selected_count, 0);
}

#[test]
fn view_uses_the_results_as_the_viewer_listing_and_go_to_file_leaves_the_list() {
    let (mut p, id, root) = searched();
    p.viewer_plugins.startup_disabled = Some(Arc::from([]));
    p.feed_find_results(id).unwrap();
    p.feed_action(PaneId::Left, FeedAction::Move(FeedMove::Last), &|_| true);
    p.feed_action(PaneId::Left, FeedAction::View(false), &|_| true);
    let launch = p.take_find_viewer().expect("a viewer opens");
    assert_eq!(launch.focused, root.join("b").join("two.log"));
    assert_eq!(
        launch.entries.len(),
        3,
        "Space and Backspace walk the results"
    );
    let commands = RefCell::new(Vec::new());
    p.feed_action(PaneId::Left, FeedAction::GoTo, &|command| {
        record(&commands, command)
    });
    assert!(p.feed(PaneId::Left).is_none(), "Go to file leaves the list");
    assert!(commands.borrow().iter().any(|command| matches!(
        command,
        Command::Navigate { pane: PaneId::Left, path, .. } if *path == root.join("b")
    )));
    assert_eq!(
        p.panes[0]
            .pending
            .as_ref()
            .and_then(|pending| pending.wanted_cursor.clone()),
        Some(root.join("b").join("two.log"))
    );
}

#[test]
fn go_to_file_of_an_archive_member_opens_its_archive_folder() {
    let (mut p, id, root) = searched();
    let commands = RefCell::new(Vec::new());
    assert!(p.reveal_find(id, "1:4", &|command| record(&commands, command)));
    let Some(Command::ValidateFindResult(request)) = commands.borrow_mut().pop() else {
        panic!("validation first");
    };
    let member = p.finds.sessions[&id].results[3].entry.clone();
    p.apply_with_sender(
        Event::FindResultValidated(Box::new(FindValidationResult {
            request,
            result: Ok(Box::new(member)),
        })),
        &|command| record(&commands, command),
    );
    let navigated = commands.borrow().iter().find_map(|command| match command {
        Command::ArchiveNavigate {
            pane: PaneId::Left,
            action: ArchiveAction::List(location),
            ..
        } => Some(location.address.clone()),
        _ => None,
    });
    let address = navigated.expect("the archive folder is listed");
    assert_eq!(address.container, root.join("bundle.zip"));
    assert_eq!(address.components, vec![b"docs".to_vec()]);
    assert!(
        !commands
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::Navigate { .. })),
        "a member is never a native path"
    );
    assert!(
        p.finds.archive_cursor.is_some(),
        "the cursor waits for the listing"
    );
}
