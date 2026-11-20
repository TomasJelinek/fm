use super::*;

fn loaded() -> (Presenter, PathBuf) {
    let root = std::env::temp_dir().join("F5 文档");
    let mut p = Presenter::new(root.clone(), root.join("target"));
    let navigation = begin(&mut p, PaneId::Left, root.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        vec![
            Entry::new(EntryId(1), root.join("first.txt"), EntryKind::File),
            Entry::new(EntryId(2), root.join("other.txt"), EntryKind::File),
        ],
    );
    p.focus_display_row(PaneId::Left, 1);
    (p, root)
}

#[test]
fn copy_and_move_confirmation_snapshot_the_committed_verification_policy() {
    for moving in [false, true] {
        for enabled in [false, true] {
            let (mut p, root) = loaded();
            p.preferences.values.copy_checksum_verification = enabled;
            if moving {
                p.request_move_dialog(PaneId::Left, &|_| true);
            } else {
                p.request_copy_dialog(PaneId::Left, &|_| true);
            }
            let token = p.copies.draft.as_ref().unwrap().token.clone();
            let commands = RefCell::new(Vec::new());
            p.confirm_copy_dialog(&token, root.join("target").to_str().unwrap(), &|command| {
                commands.borrow_mut().push(command);
                true
            });
            p.preferences.values.copy_checksum_verification = !enabled;
            let expected = TransferVerification::from_checksum_enabled(enabled);
            assert_eq!(p.copies.progress.last().unwrap().verification, expected);
            assert!(commands.borrow().iter().any(|command| matches!(command,
                Command::EnqueueSelectedCopy(request) | Command::EnqueueSelectedMove(request)
                    if request.verification == expected)));
        }
    }
}

pub(in crate::presenter) fn assert_native_destination_is_preserved(name: std::ffi::OsString) {
    let (mut p, root) = loaded();
    let target = root.join(name);
    p.panes[PaneId::Right.index()].path = target.clone();
    p.request_copy_dialog(PaneId::Left, &|_| true);
    let token = p.copies.draft.as_ref().unwrap().token.clone();
    p.confirm_copy_dialog(&token, &target.to_string_lossy(), &|command| {
        assert!(matches!(command, Command::EnqueueSelectedCopy(request) if request.destination == target));
        true
    });
}

pub(in crate::presenter) fn assert_ambiguous_destination_is_refused(name: std::ffi::OsString) {
    for moving in [false, true] {
        let (mut p, root) = loaded();
        let target = root.join(&name);
        p.panes[PaneId::Right.index()].path = target.clone();
        let source = p.panes[0].display_path().to_owned();
        let focus = p.panes[0].cursor;
        if moving {
            p.request_move_dialog(PaneId::Left, &|_| true);
        } else {
            p.request_copy_dialog(PaneId::Left, &|_| true);
        }
        let token = p.copies.draft.as_ref().unwrap().token.clone();
        let ambiguous = target.join("child").to_string_lossy().into_owned();
        p.confirm_copy_dialog(&token, &ambiguous, &|_| {
            panic!("Ambiguous destination queued work")
        });
        let draft = p
            .copies
            .draft
            .as_ref()
            .expect("Refusal must keep the draft");
        assert_eq!(draft.destination, target);
        assert!(!draft.error.is_empty());
        assert!(!p.has_live_operations());
        assert_eq!(p.panes[0].display_path(), source);
        assert_eq!(p.panes[0].cursor, focus);
        let unambiguous = root.join("explicit Unicode 日本語");
        let commands = RefCell::new(Vec::new());
        let record = |command| {
            commands.borrow_mut().push(command);
            true
        };
        p.confirm_copy_dialog(&token, unambiguous.to_str().unwrap(), &record);
        // The typed folder is checked first; it exists.
        let check = destination_check(&commands.borrow());
        assert_eq!(check.path, unambiguous);
        answer_destination_check(&mut p, &check, Ok(true), &record);
        match commands.borrow().last() {
            Some(Command::EnqueueSelectedCopy(request)) => {
                assert!(!moving);
                assert_eq!(request.destination, unambiguous);
            }
            Some(Command::EnqueueSelectedMove(request)) => {
                assert!(moving);
                assert_eq!(request.destination, unambiguous);
            }
            _ => panic!("Unexpected command"),
        }
        assert!(p.copies.draft.is_none());
    }
}

/// The destination check a confirmation asked for, the latest first.
fn destination_check(commands: &[Command]) -> DestinationCheckRequest {
    commands
        .iter()
        .rev()
        .find_map(|command| match command {
            Command::CheckTransferDestination(request) => Some(request.clone()),
            _ => None,
        })
        .expect("a typed destination is checked before the transfer starts")
}

/// Answer `check` as the backend does; the answer may start the transfer.
fn answer_destination_check(
    p: &mut Presenter,
    check: &DestinationCheckRequest,
    result: Result<bool, FsError>,
    send: &dyn Fn(Command) -> bool,
) {
    p.apply_with_sender(
        Event::TransferDestinationChecked(Box::new(DestinationCheckResult {
            request: check.request,
            path: check.path.clone(),
            result,
        })),
        send,
    );
}

/// A typed destination that is not an existing folder keeps the Copy or Move dialog
/// open with its own text and starts nothing. Once the folder exists, OK proceeds.
#[test]
fn a_typed_destination_that_is_not_an_existing_folder_is_refused_in_the_dialog() {
    for moving in [false, true] {
        for result in [
            Err(io::ErrorKind::NotFound),
            Err(io::ErrorKind::NotADirectory),
            Ok(false),
        ] {
            let what = format!("moving {moving}, {result:?}");
            let (mut p, root) = loaded();
            if moving {
                p.request_move_dialog(PaneId::Left, &|_| true);
            } else {
                p.request_copy_dialog(PaneId::Left, &|_| true);
            }
            let token = p.copies.draft.as_ref().unwrap().token.clone();
            let missing = root.join("missing");
            let commands = RefCell::new(Vec::new());
            let record = |command| {
                commands.borrow_mut().push(command);
                true
            };
            p.confirm_copy_dialog(&token, &missing.to_string_lossy(), &record);
            let check = destination_check(&commands.borrow());
            assert_eq!(check.path, missing);
            assert_eq!(commands.borrow().len(), 1, "only the check, no transfer");
            assert!(p.copies.draft.as_ref().unwrap().error.is_empty());
            // OK again while the folder is checked asks nothing more.
            p.confirm_copy_dialog(&token, &missing.to_string_lossy(), &record);
            assert_eq!(commands.borrow().len(), 1);
            answer_destination_check(
                &mut p,
                &check,
                result.map_err(|kind| FsError::from_io("check", &missing, kind.into())),
                &record,
            );
            assert_eq!(commands.borrow().len(), 1, "{what}: no transfer");
            assert!(!p.has_live_operations(), "{what}");
            assert_eq!(
                p.copies
                    .draft
                    .as_ref()
                    .expect("the dialog stays open")
                    .error,
                crate::presenter::copy_dialog::DESTINATION_FOLDER_REQUIRED,
                "{what}"
            );
            // The folder exists now: the next OK checks again and starts the transfer.
            p.confirm_copy_dialog(&token, &missing.to_string_lossy(), &record);
            let again = destination_check(&commands.borrow());
            assert_ne!(again.request, check.request);
            answer_destination_check(&mut p, &again, Ok(true), &record);
            match commands.borrow().last() {
                Some(Command::EnqueueSelectedCopy(request)) if !moving => {
                    assert_eq!(request.destination, missing);
                }
                Some(Command::EnqueueSelectedMove(request)) if moving => {
                    assert_eq!(request.destination, missing);
                }
                _ => panic!("OK starts the transfer"),
            }
            assert!(p.copies.draft.is_none());
            assert!(p.has_live_operations());
        }
    }
}

/// Only the answer for the folder being checked counts: a newer OK cancels the older
/// check, whose late answer starts nothing, and closing the dialog cancels its
/// check. When the backend cannot tell, the transfer starts and reports the problem.
#[test]
fn only_the_current_destination_check_counts() {
    let (mut p, root) = loaded();
    p.request_copy_dialog(PaneId::Left, &|_| true);
    let token = p.copies.draft.as_ref().unwrap().token.clone();
    let commands = RefCell::new(Vec::new());
    let record = |command| {
        commands.borrow_mut().push(command);
        true
    };
    p.confirm_copy_dialog(&token, &root.join("first").to_string_lossy(), &record);
    let older = destination_check(&commands.borrow());
    let second = root.join("second");
    p.confirm_copy_dialog(&token, &second.to_string_lossy(), &record);
    let newer = destination_check(&commands.borrow());
    assert_eq!(newer.path, second);
    assert!(
        older.cancel.is_cancelled(),
        "a newer OK cancels the older check"
    );
    answer_destination_check(&mut p, &older, Ok(true), &record);
    assert!(p.copies.draft.is_some());
    assert!(
        !p.has_live_operations(),
        "a superseded answer starts nothing"
    );
    let denied = FsError::from_io("check", &second, io::ErrorKind::PermissionDenied.into());
    answer_destination_check(&mut p, &newer, Err(denied), &record);
    assert!(matches!(
        commands.borrow().last(),
        Some(Command::EnqueueSelectedCopy(request)) if request.destination == second
    ));
    assert!(p.copies.draft.is_none());

    let (mut p, root) = loaded();
    p.request_copy_dialog(PaneId::Left, &|_| true);
    let token = p.copies.draft.as_ref().unwrap().token.clone();
    let commands = RefCell::new(Vec::new());
    let record = |command| {
        commands.borrow_mut().push(command);
        true
    };
    p.confirm_copy_dialog(&token, &root.join("late").to_string_lossy(), &record);
    let check = destination_check(&commands.borrow());
    p.cancel_copy_dialog(&token);
    assert!(
        check.cancel.is_cancelled(),
        "closing the dialog cancels its check"
    );
    answer_destination_check(&mut p, &check, Ok(true), &record);
    assert_eq!(commands.borrow().len(), 1, "a late answer starts nothing");
    assert!(!p.has_live_operations());
}

#[test]
fn copy_dialog_captures_selection_and_other_panel_path_before_selection_finishes() {
    let (mut p, root) = loaded();
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    p.bulk_selection(PaneId::Left, SelectionOperation::All, &send);
    let Command::Selection(request) = commands.borrow_mut().pop().unwrap() else {
        unreachable!()
    };
    p.request_copy_dialog(PaneId::Left, &send);
    assert!(p.copies.draft.is_none());
    p.navigate(PaneId::Right, root.join("changed target"), None, &send);
    p.focus_display_row(PaneId::Left, 2);
    let selection =
        Arc::new(SelectionSnapshot::from_bits(request.view.clone().unwrap(), vec![3]).unwrap());
    p.apply_with_sender(
        Event::SelectionChanged {
            pane: PaneId::Left,
            navigation: request.navigation,
            revision: request.revision,
            selection: selection.clone(),
            deferred: Default::default(),
        },
        &send,
    );
    let draft = p.copies.draft.as_ref().unwrap();
    let crate::presenter::copy_dialog::CopyDraftInput::Native(files) = &draft.input else {
        panic!("native draft")
    };
    assert!(Arc::ptr_eq(&files.selection, &selection));
    assert_eq!(draft.destination, root.join("target"));
    assert_eq!(files.fallback, Some(root.join("first.txt")));
    let token = draft.token.clone();
    let destination = draft.destination.to_string_lossy().into_owned();
    p.navigate(PaneId::Left, root.join("new source"), None, &send);
    commands.borrow_mut().clear();
    p.confirm_copy_dialog(&token, &destination, &send);
    p.confirm_copy_dialog(&token, &destination, &send);
    let commands = commands.borrow();
    assert_eq!(commands.len(), 1);
    let Command::EnqueueSelectedCopy(copy) = &commands[0] else {
        panic!("direct copy, no clipboard")
    };
    assert!(Arc::ptr_eq(&copy.files.selection, &selection));
    assert_eq!(copy.destination, root.join("target"));
    assert!(p.copies.draft.is_none());
    assert!(p.has_live_operations());
}

#[test]
fn copy_dialog_cancel_and_invalid_or_stale_confirmation_never_queue_work() {
    let (mut p, _) = loaded();
    p.request_copy_dialog(PaneId::Left, &|_| {
        panic!("opening the dialog performs no I/O")
    });
    let token = p.copies.draft.as_ref().unwrap().token.clone();
    let no_send = |_: Command| -> bool { panic!("no copy may be queued") };
    p.confirm_copy_dialog("stale", "", &no_send);
    p.confirm_copy_dialog(&token, "", &no_send);
    assert!(!p.copies.draft.as_ref().unwrap().error.is_empty());
    p.cancel_copy_dialog("stale");
    assert!(p.copies.draft.is_some());
    drop(p.take_retired());
    p.cancel_copy_dialog(&token);
    assert!(
        !p.take_retired().is_empty(),
        "cancel retires the captured draft off the UI thread"
    );
    p.confirm_copy_dialog(&token, "anything", &no_send);
    assert!(p.copies.draft.is_none());
    assert!(!p.has_live_operations());
}

#[test]
fn copy_dialog_rejected_admission_keeps_the_draft_for_retry() {
    let (mut p, root) = loaded();
    p.request_copy_dialog(PaneId::Left, &|_| true);
    let token = p.copies.draft.as_ref().unwrap().token.clone();
    let target = root.join("edited destination");
    // A refused check keeps the draft for another OK, as a refused transfer does.
    p.confirm_copy_dialog(&token, &target.to_string_lossy(), &|_| false);
    assert!(p.copies.draft.is_some());
    assert!(!p.has_live_operations());
    let commands = RefCell::new(Vec::new());
    let record = |command| {
        commands.borrow_mut().push(command);
        true
    };
    p.confirm_copy_dialog(&token, &target.to_string_lossy(), &record);
    let check = destination_check(&commands.borrow());
    answer_destination_check(&mut p, &check, Ok(true), &|_| false);
    assert!(
        p.copies.draft.is_some(),
        "a refused transfer keeps the draft"
    );
    assert!(!p.has_live_operations());
    p.confirm_copy_dialog(&token, &target.to_string_lossy(), &record);
    let check = destination_check(&commands.borrow());
    answer_destination_check(&mut p, &check, Ok(true), &record);
    assert!(matches!(
        commands.borrow().last(),
        Some(Command::EnqueueSelectedCopy(request)) if request.destination == target
    ));
    assert!(p.copies.draft.is_none());
}

#[test]
fn move_clipboard_opens_a_confirmation_without_enqueuing_or_copying() {
    let (mut p, root) = loaded();
    let destination = root.join("captured destination 文档");
    let clipboard = FileClipboard {
        paths: vec![root.join("original.txt")],
        intent: FileClipboardIntent::Move,
    };
    p.start_copy(clipboard, destination.clone(), &|_| {
        panic!("Move clipboard must await explicit confirmation")
    });
    assert!(
        p.copies.draft.is_some(),
        "Move clipboard must open the destination confirmation"
    );
    assert_eq!(p.copies.draft.as_ref().unwrap().destination, destination);
    let token = p.copies.draft.as_ref().unwrap().token.clone();
    p.cancel_copy_dialog(&token);
    assert!(p.copies.draft.is_none());
    assert!(!p.has_live_operations());
}

#[test]
fn f6_move_captures_selection_and_destination_and_confirms_once() {
    let (mut p, root) = loaded();
    p.request_move_dialog(PaneId::Left, &|_| panic!("Opening F6 performs no I/O"));
    let draft = p.copies.draft.as_ref().unwrap();
    assert_eq!(draft.intent, TransferIntent::Move);
    let token = draft.token.clone();
    let destination = draft.destination.clone();
    p.focus_display_row(PaneId::Left, 2);
    p.panes[PaneId::Right.index()].path = root.join("changed destination");
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    p.confirm_copy_dialog(&token, &destination.to_string_lossy(), &send);
    p.confirm_copy_dialog(&token, &destination.to_string_lossy(), &send);
    let commands = commands.borrow();
    let [Command::EnqueueSelectedMove(request)] = commands.as_slice() else {
        panic!("F6 must submit one direct selected Move");
    };
    assert_eq!(request.destination, root.join("target"));
    assert_eq!(request.files.fallback, Some(root.join("first.txt")));
    assert_eq!(
        p.copies.progress.last().unwrap().intent,
        TransferIntent::Move
    );
}

#[test]
fn clipboard_move_retains_shared_raw_paths_and_original_destination_after_busy_retry() {
    let (mut p, root) = loaded();
    let paths = vec![root.join("original 文档.txt")];
    let destination = root.join("target");
    p.start_copy(
        FileClipboard {
            paths: paths.clone(),
            intent: FileClipboardIntent::Move,
        },
        destination.clone(),
        &|_| panic!("confirmation first"),
    );
    let draft = p.copies.draft.as_ref().unwrap();
    let crate::presenter::copy_dialog::CopyDraftInput::Clipboard(files) = &draft.input else {
        panic!("shared raw clipboard draft");
    };
    let captured = Arc::downgrade(files);
    let token = draft.token.clone();
    p.panes[PaneId::Right.index()].path = root.join("other");
    p.confirm_copy_dialog(&token, &destination.to_string_lossy(), &|_| false);
    assert!(p.copies.draft.is_some());
    assert_eq!(
        captured.strong_count(),
        1,
        "the retained draft must be the independent raw-payload owner after queue refusal"
    );
    assert!(!p.has_live_operations());
    p.confirm_copy_dialog(&token, &destination.to_string_lossy(), &|command| {
        let Command::EnqueueClipboardMove(request) = command else {
            panic!("shared clipboard Move route");
        };
        assert!(Arc::ptr_eq(
            &request.clipboard,
            &captured.upgrade().unwrap()
        ));
        assert_eq!(request.clipboard.paths, paths);
        assert_eq!(request.destination, destination);
        true
    });
    assert!(p.copies.draft.is_none());
    assert_eq!(
        p.copies.progress.last().unwrap().intent,
        TransferIntent::Move
    );
}

#[test]
fn f6_waits_for_the_exact_pending_selection_revision() {
    let (mut p, root) = loaded();
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    p.bulk_selection(PaneId::Left, SelectionOperation::All, &send);
    let Command::Selection(request) = commands.borrow_mut().pop().unwrap() else {
        panic!("selection request");
    };
    p.request_move_dialog(PaneId::Left, &send);
    assert!(p.copies.draft.is_none());
    p.panes[PaneId::Right.index()].path = root.join("new target");
    let selection =
        Arc::new(SelectionSnapshot::from_bits(request.view.clone().unwrap(), vec![3]).unwrap());
    p.apply_with_sender(
        Event::SelectionChanged {
            pane: PaneId::Left,
            navigation: request.navigation,
            revision: request.revision,
            selection: selection.clone(),
            deferred: Default::default(),
        },
        &send,
    );
    let draft = p.copies.draft.as_ref().unwrap();
    assert_eq!(draft.destination, root.join("target"));
    let crate::presenter::copy_dialog::CopyDraftInput::Native(files) = &draft.input else {
        panic!("native selection");
    };
    assert!(Arc::ptr_eq(&files.selection, &selection));
    assert_eq!(draft.intent, TransferIntent::Move);
}

#[test]
fn required_modified_time_preserves_captured_copy_and_move_after_queue_retry() {
    for moving in [false, true] {
        let (mut p, root) = loaded();
        if moving {
            p.request_move_dialog(PaneId::Left, &|_| true);
        } else {
            p.request_copy_dialog(PaneId::Left, &|_| true);
        }
        let draft = p.copies.draft.as_ref().unwrap();
        assert!(draft.metadata_available());
        let token = draft.token.clone();
        let target = draft.destination.clone();
        let crate::presenter::copy_dialog::CopyDraftInput::Native(files) = &draft.input else {
            panic!("native capture");
        };
        let selection = files.selection.clone();
        let view = files.view.clone();
        p.focus_display_row(PaneId::Left, 2);
        p.confirm_copy_dialog_metadata(
            &token,
            &target.to_string_lossy(),
            TransferMetadataPolicy::RequireModifiedTime,
            &|_| false,
        );
        assert!(p.copies.draft.is_some());
        assert!(!p.has_live_operations());
        let queued = RefCell::new(Vec::new());
        let send = |command| {
            queued.borrow_mut().push(command);
            true
        };
        p.confirm_copy_dialog_metadata(
            "stale",
            &target.to_string_lossy(),
            TransferMetadataPolicy::RequireModifiedTime,
            &send,
        );
        p.confirm_copy_dialog_metadata(
            &token,
            &target.to_string_lossy(),
            TransferMetadataPolicy::RequireModifiedTime,
            &send,
        );
        let queued = queued.borrow();
        let [Command::EnqueueTransferWithMetadata(request)] = queued.as_slice() else {
            panic!("exactly one required metadata command");
        };
        assert_eq!(
            request.metadata,
            TransferMetadataPolicy::RequireModifiedTime
        );
        assert_eq!(
            request.intent,
            if moving {
                TransferIntent::Move
            } else {
                TransferIntent::Copy
            }
        );
        let MetadataTransferInput::Selected(request) = &request.input else {
            panic!("selected capture");
        };
        assert!(Arc::ptr_eq(&request.files.selection, &selection));
        assert!(Arc::ptr_eq(&request.files.view, &view));
        assert_eq!(request.files.fallback, Some(root.join("first.txt")));
        assert_eq!(request.destination, target);
        assert!(p.copies.draft.is_none());
    }
}

#[test]
fn required_metadata_refuses_known_folders_and_over_budget_without_downgrading() {
    for (kind, count, selected) in [
        (EntryKind::Directory, 1usize, false),
        (EntryKind::DirectoryLink, 1, false),
        (EntryKind::Directory, 1, true),
        (EntryKind::File, 65, true),
    ] {
        let (mut p, root) = loaded();
        p.request_copy_dialog(PaneId::Left, &|_| true);
        let entries = (0..count)
            .map(|id| Entry::new(EntryId(id as u64), root.join(format!("item-{id}")), kind))
            .collect::<Vec<_>>();
        let view = Arc::new(DirectoryIndex::from_entries(&entries));
        let selection = if selected {
            let mut bits = vec![u64::MAX; count.div_ceil(64)];
            if count % 64 != 0 {
                *bits.last_mut().unwrap() = (1u64 << (count % 64)) - 1;
            }
            Arc::new(SelectionSnapshot::from_bits(view.clone(), bits).unwrap())
        } else {
            Arc::default()
        };
        let draft = p.copies.draft.as_mut().unwrap();
        draft.input = crate::presenter::copy_dialog::CopyDraftInput::Native(SelectedFiles {
            selection,
            view,
            fallback: Some(entries[0].path.clone()),
        });
        assert!(!draft.metadata_available());
        let token = draft.token.clone();
        let target = draft.destination.clone();
        p.confirm_copy_dialog_metadata(
            &token,
            &target.to_string_lossy(),
            TransferMetadataPolicy::RequireModifiedTime,
            &|_| panic!("an unsupported required policy must never enqueue content-only work"),
        );
        assert!(!p.copies.draft.as_ref().unwrap().error.is_empty());
        assert!(!p.has_live_operations());
    }
}

#[test]
fn clipboard_required_metadata_retains_original_raw_payload_and_intent() {
    for start_policy in [
        TransferStartPolicy::Immediate,
        TransferStartPolicy::WaitForEarlier,
    ] {
        let (mut p, root) = loaded();
        let target = root.join("target");
        p.start_copy(
            FileClipboard {
                paths: vec![root.join("source 文档")],
                intent: FileClipboardIntent::Move,
            },
            target.clone(),
            &|_| panic!("confirmation first"),
        );
        let draft = p.copies.draft.as_ref().unwrap();
        assert!(draft.metadata_available());
        let crate::presenter::copy_dialog::CopyDraftInput::Clipboard(files) = &draft.input else {
            panic!("clipboard capture");
        };
        let owner = Arc::downgrade(files);
        let token = draft.token.clone();
        p.confirm_copy_dialog_options(
            &token,
            &target.to_string_lossy(),
            TransferMetadataPolicy::RequireModifiedTime,
            start_policy,
            &|_| false,
        );
        assert_eq!(owner.strong_count(), 1);
        p.confirm_copy_dialog_options(
            &token,
            &target.to_string_lossy(),
            TransferMetadataPolicy::RequireModifiedTime,
            start_policy,
            &|command| {
                let Command::EnqueueTransferWithMetadata(request) = command else {
                    panic!("metadata command");
                };
                assert_eq!(request.intent, TransferIntent::Move);
                assert_eq!(
                    request.metadata,
                    TransferMetadataPolicy::RequireModifiedTime
                );
                let MetadataTransferInput::Clipboard(request) = request.input else {
                    panic!("clipboard metadata");
                };
                assert_eq!(request.start_policy, start_policy);
                assert!(Arc::ptr_eq(&request.clipboard, &owner.upgrade().unwrap()));
                assert_eq!(request.destination, target);
                true
            },
        );
        assert!(p.copies.draft.is_none());
    }
}

#[test]
fn archive_required_metadata_refuses_forged_option_without_native_access() {
    let (mut p, root) = loaded();
    p.request_copy_dialog(PaneId::Left, &|_| true);
    let draft = p.copies.draft.as_mut().unwrap();
    draft.input =
        crate::presenter::copy_dialog::CopyDraftInput::Archive(Box::new(ArchiveCopyRequest {
            start_policy: TransferStartPolicy::Immediate,
            verification: midnight_core::TransferVerification::default(),
            id: OperationId(0),
            location: ArchiveLocation::unavailable(ArchiveAddress {
                format: None,
                provider: "fake.archive".into(),
                container: root.join("source.zip"),
                source_version: vec![1],
                components: vec![],
            }),
            entries: Arc::from([]),
            view: Arc::default(),
            selected: Arc::default(),
            fallback: Some(EntryId(2)),
            destination: root.join("target"),
        }));
    assert!(!draft.metadata_available());
    let token = draft.token.clone();
    let target = draft.destination.clone();
    p.confirm_copy_dialog_metadata(
        &token,
        &target.to_string_lossy(),
        TransferMetadataPolicy::RequireModifiedTime,
        &|_| panic!("archive required metadata must not silently downgrade"),
    );
    assert!(p.copies.draft.is_some());
    assert!(!p.copies.draft.as_ref().unwrap().error.is_empty());
    assert!(!p.has_live_operations());
}

#[test]
fn selected_draft_freezes_chosen_start_policy_after_rejected_admission() {
    for moving in [false, true] {
        for metadata in [
            TransferMetadataPolicy::ContentOnly,
            TransferMetadataPolicy::RequireModifiedTime,
        ] {
            for start_policy in [
                TransferStartPolicy::Immediate,
                TransferStartPolicy::WaitForEarlier,
            ] {
                let (mut p, root) = loaded();
                if moving {
                    p.request_move_dialog(PaneId::Left, &|_| true);
                } else {
                    p.request_copy_dialog(PaneId::Left, &|_| true);
                }
                let draft = p.copies.draft.as_ref().unwrap();
                let token = draft.token.clone();
                let destination = draft.destination.clone();
                let crate::presenter::copy_dialog::CopyDraftInput::Native(capture) = &draft.input
                else {
                    panic!("native capture");
                };
                let selection = capture.selection.clone();
                p.confirm_copy_dialog_options(
                    &token,
                    &destination.to_string_lossy(),
                    metadata,
                    start_policy,
                    &|_| false,
                );
                assert_eq!(p.copies.draft.as_ref().unwrap().token, token);
                assert!(!p.has_live_operations());
                p.panes[1].path = root.join("later target");
                let sent = RefCell::new(Vec::new());
                let send = |command| {
                    sent.borrow_mut().push(command);
                    true
                };
                p.confirm_copy_dialog_options(
                    &token,
                    &destination.to_string_lossy(),
                    metadata,
                    start_policy,
                    &send,
                );
                p.confirm_copy_dialog_options(
                    &token,
                    &destination.to_string_lossy(),
                    metadata,
                    start_policy,
                    &send,
                );
                let sent = sent.borrow();
                let [command] = sent.as_slice() else {
                    panic!("one captured command");
                };
                let leaf = match command {
                    Command::EnqueueSelectedCopy(leaf) if !moving => leaf,
                    Command::EnqueueSelectedMove(leaf) if moving => leaf,
                    Command::EnqueueTransferWithMetadata(wrapper) => {
                        assert_eq!(wrapper.metadata, metadata);
                        let MetadataTransferInput::Selected(leaf) = &wrapper.input else {
                            panic!("selected metadata leaf");
                        };
                        leaf
                    }
                    _ => panic!("unexpected transfer route"),
                };
                assert_eq!(leaf.start_policy, start_policy);
                assert_eq!(leaf.destination, destination);
                assert!(Arc::ptr_eq(&leaf.files.selection, &selection));
            }
        }
    }
}

#[test]
fn clipboard_copy_without_a_dialog_always_submits_immediately() {
    let (mut p, root) = loaded();
    p.start_copy(
        FileClipboard {
            paths: vec![root.join("raw 文档")],
            intent: FileClipboardIntent::Copy,
        },
        root.join("target"),
        &|command| {
            let Command::EnqueueCopy(leaf) = command else {
                panic!("direct clipboard copy");
            };
            assert_eq!(leaf.start_policy, TransferStartPolicy::Immediate);
            true
        },
    );
    assert!(p.copies.draft.is_none());
}
