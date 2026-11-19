use super::*;
use crate::presenter::tests::assert_late_event_is_only_retired;

fn setup(send: &dyn Fn(Command) -> bool) -> (Presenter, ChecksumJobId) {
    let root = std::env::temp_dir();
    let mut p = Presenter::new(root.clone(), root.clone());
    let files = SelectedFiles {
        selection: Arc::new(SelectionSnapshot::default()),
        view: Arc::new(DirectoryIndex::default()),
        fallback: Some(root.join("Žluťoučký 日本語.txt")),
    };
    let id = p
        .open_captured_checksums(PaneId::Left, root, files, send)
        .unwrap();
    // Opening calculates nothing; the tests press Calculate as the user would.
    let algorithms = p.checksums.sessions[&id].algorithms;
    p.start_checksums(id, algorithms, false, send);
    (p, id)
}

#[test]
fn verification_uses_manifest_folder_and_does_not_retarget_with_panel_navigation() {
    let commands = std::cell::RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    let (mut p, id) = setup(&send);
    finish(&mut p, id, ChecksumCompletion::Completed);
    let directory = p.checksums.sessions[&id].directory.clone();
    p.panes[0].path.push("other");
    assert!(p.start_checksum_verification(id, "日本語.sha256", &send));
    let sent = commands.borrow();
    let Command::VerifyChecksumManifest(request) = sent.last().unwrap() else {
        panic!()
    };
    assert_eq!(request.manifest, directory.join("日本語.sha256"));
    assert_eq!(request.generation, ChecksumGeneration(2));
    assert!(p.checksums.sessions[&id].verification.is_some());
    assert!(
        p.checksums.sessions[&id]
            .export_counts(ChecksumAlgorithm::Sha256)
            .is_none()
    );
    assert!(!p.start_checksum_verification(id, "other.md5", &send));
}

#[test]
fn checksum_picker_returns_the_picked_path_without_starting_work_and_close_retires_late_reply() {
    let commands = std::cell::RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    let (mut p, id) = setup(&send);
    finish(&mut p, id, ChecksumCompletion::Completed);
    p.request_checksum_picker(
        id,
        true,
        ChecksumAlgorithm::Sha256,
        PathPickerOwner::None,
        &send,
    );
    let request = match commands.borrow().last().unwrap() {
        Command::PickPath { request, input } => {
            assert_eq!(input.kind, PathPickerKind::ChecksumOpen);
            *request
        }
        _ => panic!(),
    };
    let path = std::env::temp_dir().join("checksums 日本語.sfv");
    commands.borrow_mut().clear();
    p.apply(Event::PathPicked {
        request,
        result: Ok(Some(path.clone())),
    });
    assert_eq!(p.checksums.sessions[&id].manifest_suggested, path);
    assert!(!p.checksums.sessions[&id].running);
    assert!(commands.borrow().is_empty());
    p.request_checksum_picker(
        id,
        false,
        ChecksumAlgorithm::Crc32,
        PathPickerOwner::None,
        &send,
    );
    let late = match commands.borrow().last().unwrap() {
        Command::PickPath { request, input } => {
            assert_eq!(
                input.kind,
                PathPickerKind::ChecksumSave(ChecksumAlgorithm::Crc32)
            );
            *request
        }
        _ => panic!(),
    };
    assert!(p.close_checksums(id, &send));
    assert!(commands.borrow().iter().any(|command| matches!(command,
        Command::CancelPathPicker { request } if *request == late)));
    assert_late_event_is_only_retired(&mut p, |p| {
        p.apply(Event::PathPicked {
            request: late,
            result: Ok(Some(path)),
        })
    });
    assert!(!p.checksums.sessions.contains_key(&id));
}

#[test]
fn verify_queue_refusal_preserves_previous_results_and_generation() {
    let (mut p, id) = setup(&|_| true);
    finish(&mut p, id, ChecksumCompletion::Completed);
    assert!(!p.start_checksum_verification(id, "checksums.sfv", &|_| false));
    let s = &p.checksums.sessions[&id];
    assert_eq!(s.generation, ChecksumGeneration(1));
    assert!(s.run.is_some());
    assert!(s.verification.is_none());
    assert!(!s.running);
}

#[test]
fn generated_filename_follows_preferred_format_and_recalculation() {
    let (mut p, id) = setup(&|_| true);
    finish(&mut p, id, ChecksumCompletion::Completed);
    let choices =
        ChecksumAlgorithms::new(&[ChecksumAlgorithm::Md5, ChecksumAlgorithm::Sha256]).unwrap();
    assert!(p.start_checksums(id, choices, false, &|_| true));
    assert_eq!(
        p.checksums.sessions[&id].suggested.extension().unwrap(),
        "sha256"
    );
    finish(&mut p, id, ChecksumCompletion::Completed);
    let choices = ChecksumAlgorithms::new(&[ChecksumAlgorithm::Crc32]).unwrap();
    assert!(p.start_checksums(id, choices, false, &|_| true));
    assert_eq!(
        p.checksums.sessions[&id].suggested.extension().unwrap(),
        "sfv"
    );
}

#[test]
fn focused_checksum_sidecar_prefills_verification_without_hashing_the_manifest() {
    let root = std::env::temp_dir();
    let manifest = root.join("日本語.MD5");
    let commands = std::cell::RefCell::new(Vec::new());
    let mut p = Presenter::new(root.clone(), root.clone());
    let id = p
        .open_captured_checksums(
            PaneId::Left,
            root,
            SelectedFiles {
                selection: Arc::new(SelectionSnapshot::default()),
                view: Arc::new(DirectoryIndex::default()),
                fallback: Some(manifest.clone()),
            },
            &|command| {
                commands.borrow_mut().push(command);
                true
            },
        )
        .unwrap();
    let s = &p.checksums.sessions[&id];
    assert!(s.verify_initial);
    assert_eq!(s.manifest_suggested, manifest);
    assert_eq!(s.generation, ChecksumGeneration(0));
    assert!(!s.running);
    assert!(commands.borrow().is_empty());
}

/// Finish the session's current run with one calculated SHA-256 row.
pub(super) fn finish(p: &mut Presenter, id: ChecksumJobId, completion: ChecksumCompletion) {
    let generation = p.checksums.sessions[&id].generation;
    let retention = ChecksumRetention::new(());
    let row = Arc::new(ChecksumRow {
        id: ChecksumRowId(1),
        path: p.checksums.sessions[&id].files.fallback.clone().unwrap(),
        identity: None,
        kind: Some(EntryKind::File),
        state: ChecksumRowState::Calculated,
        bytes_read: 3,
        digests: vec![ChecksumDigest {
            algorithm: ChecksumAlgorithm::Sha256,
            bytes: vec![0x42; 32],
        }],
        error: None,
        retention: retention.clone(),
    });
    p.apply(Event::Checksum(Box::new(ChecksumEvent {
        job: id,
        generation,
        update: ChecksumUpdate::Finished(Arc::new(ChecksumRun {
            job: id,
            generation,
            algorithms: ChecksumAlgorithms::default(),
            captured_roots: vec![row.path.clone()].into(),
            rows: vec![row].into(),
            summary: ChecksumSummary {
                completion,
                progress: ChecksumProgress {
                    discovered: 1,
                    calculated: 1,
                    bytes_read: 3,
                    ..Default::default()
                },
                reached_budgets: vec![],
                diagnostics: vec![],
            },
            retention,
        })),
    })));
}

#[test]
fn checksum_queue_refusal_does_not_commit_start_stop_or_close() {
    let (mut p, id) = setup(&|_| false);
    assert_eq!(p.checksums.sessions[&id].generation, ChecksumGeneration(0));
    assert!(!p.checksums.sessions[&id].running);
    assert!(!p.checksums.sessions[&id].error.is_empty());
    assert!(p.start_checksums(id, ChecksumAlgorithms::default(), false, &|_| true));
    p.stop_checksums(id, &|_| false);
    assert!(!p.checksums.sessions[&id].stopping);
    assert!(!p.close_checksums(id, &|_| false));
    assert!(p.checksums.sessions.contains_key(&id));
    drop(p.take_retired());
    assert!(p.close_checksums(id, &|_| true));
    assert!(!p.checksums.sessions.contains_key(&id));
    assert!(
        !p.take_retired().is_empty(),
        "the admitted close retires the session off the UI thread"
    );
}

#[test]
fn checksum_recalculate_keeps_capture_and_old_row_tokens_cannot_copy_new_results() {
    let commands = std::cell::RefCell::new(Vec::new());
    let send = |c| {
        commands.borrow_mut().push(c);
        true
    };
    let (mut p, id) = setup(&send);
    let captured = p.checksums.sessions[&id].files.fallback.clone();
    finish(&mut p, id, ChecksumCompletion::Completed);
    p.copy_checksum(id, "1:1", ChecksumAlgorithm::Sha256, false, &send);
    assert!(
        commands
            .borrow()
            .iter()
            .any(|c| matches!(c, Command::CopyText { text, .. } if text == &"42".repeat(32)))
    );
    commands.borrow_mut().clear();
    p.panes[0].path.push("different-folder");
    assert!(p.start_checksums(id, ChecksumAlgorithms::default(), true, &send));
    let sent = commands.borrow();
    let Command::StartSelectedChecksums(request) = sent.last().unwrap() else {
        panic!()
    };
    assert_eq!(request.files.fallback, captured);
    assert!(request.recursive);
    drop(sent);
    finish(&mut p, id, ChecksumCompletion::Completed);
    commands.borrow_mut().clear();
    p.copy_checksum(id, "1:1", ChecksumAlgorithm::Sha256, false, &send);
    assert!(commands.borrow().is_empty());
    p.copy_checksum(id, "2:1", ChecksumAlgorithm::Md5, false, &send);
    assert!(
        commands.borrow().is_empty(),
        "uncalculated algorithm cannot be copied"
    );
}

#[test]
fn checksum_full_row_copy_ignores_uncalculated_output_selector() {
    let (mut p, id) = setup(&|_| true);
    finish(&mut p, id, ChecksumCompletion::Completed);
    let texts = std::cell::RefCell::new(Vec::new());
    p.copy_checksum(id, "1:1", ChecksumAlgorithm::Md5, true, &|command| {
        if let Command::CopyText { text, .. } = command {
            texts.borrow_mut().push(text);
        }
        true
    });
    assert_eq!(texts.borrow().len(), 1);
    assert!(texts.borrow()[0].contains("SHA-256: "));
    assert!(texts.borrow()[0].contains(&"42".repeat(32)));
    assert!(!texts.borrow()[0].contains("MD5:"));
}

#[test]
fn manifest_filename_uses_captured_folder_and_invalid_drafts_never_enqueue() {
    let (mut p, id) = setup(&|_| true);
    finish(&mut p, id, ChecksumCompletion::Completed);
    let expected = p.checksums.sessions[&id]
        .suggested
        .with_file_name("export 日本語.sha256");
    p.panes[0].path.push("navigated-away");
    let commands = std::cell::RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    let form = |destination: &str| checksums::ChecksumSaveForm {
        generation: 1,
        algorithm: ChecksumAlgorithm::Sha256,
        destination: destination.into(),
        acknowledge: false, overwrite: false,
    };
    assert!(!p.save_checksum_manifest(id, form("../outside.sha256"), &send));
    assert!(commands.borrow().is_empty());
    assert!(p.checksums.sessions[&id].save.is_none());
    assert!(!p.checksums.sessions[&id].error.is_empty());
    assert!(p.save_checksum_manifest(id, form("export 日本語.sha256"), &send));
    let sent = commands.borrow();
    assert_eq!(sent.len(), 1);
    let Command::SaveChecksumManifest { request, .. } = &sent[0] else {
        panic!("Only the valid captured destination may enter the manifest queue");
    };
    assert_eq!(request.destination, expected);
    assert!(p.checksums.sessions[&id].error.is_empty());
}

#[test]
fn partial_checksum_save_requires_exact_current_run_acknowledgement() {
    let (mut p, id) = setup(&|_| true);
    finish(&mut p, id, ChecksumCompletion::Incomplete);
    let path = std::env::temp_dir().join("checksums.sha256");
    let text = path.to_string_lossy();
    let commands = std::cell::RefCell::new(Vec::new());
    let send = |c| {
        commands.borrow_mut().push(c);
        true
    };
    let form = |acknowledge| checksums::ChecksumSaveForm {
        generation: 1,
        algorithm: ChecksumAlgorithm::Sha256,
        destination: text.to_string(),
        acknowledge, overwrite: false,
    };
    assert!(!p.save_checksum_manifest(id, form(false), &send));
    assert!(commands.borrow().is_empty());
    assert!(p.save_checksum_manifest(id, form(true), &send));
    let sent = commands.borrow();
    let Command::SaveChecksumManifest { request, .. } = sent.last().unwrap() else {
        panic!()
    };
    assert_eq!(request.destination, path);
    assert_eq!(
        request.acknowledge_partial,
        Some(ChecksumPartialAcknowledgement {
            included: 1,
            excluded: 0,
            incomplete_discovery: true,
        })
    );
    drop(sent);
    assert!(
        !p.start_checksums(id, ChecksumAlgorithms::default(), false, &send),
        "a save keeps ownership of its exact generation until it finishes"
    );
}

#[test]
fn late_checksum_events_do_not_reopen_released_windows() {
    let (mut p, id) = setup(&|_| true);
    assert!(p.close_checksums(id, &|_| true));
    assert_late_event_is_only_retired(&mut p, |p| {
        p.apply(Event::Checksum(Box::new(ChecksumEvent {
            job: id,
            generation: ChecksumGeneration(1),
            update: ChecksumUpdate::Started,
        })))
    });
    assert!(p.checksums.sessions.is_empty());
}

#[test]
fn opening_calculates_nothing_and_a_selected_checksum_file_opens_verification() {
    let root = std::env::temp_dir();
    let mut p = Presenter::new(root.clone(), root.clone());
    let sent = std::cell::RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    let files = |name: &str| SelectedFiles {
        selection: Arc::new(SelectionSnapshot::default()),
        view: Arc::new(DirectoryIndex::default()),
        fallback: Some(root.join(name)),
    };
    let plain = p
        .open_captured_checksums(PaneId::Left, root.clone(), files("photo.jpg"), &send)
        .unwrap();
    assert!(!p.checksums.sessions[&plain].running);
    assert!(!p.checksums.sessions[&plain].verify_initial);
    let manifest = p
        .open_captured_checksums(PaneId::Left, root.clone(), files("photos.SHA256"), &send)
        .unwrap();
    assert!(p.checksums.sessions[&manifest].verify_initial);
    assert_eq!(
        p.checksums.sessions[&manifest].manifest_suggested,
        root.join("photos.SHA256")
    );
    assert!(
        sent.borrow().is_empty(),
        "nothing is calculated or verified until the user asks"
    );
}

#[test]
fn an_existing_manifest_is_offered_for_replacement_instead_of_failing() {
    let sent = std::cell::RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    let (mut p, id) = setup(&send);
    finish(&mut p, id, ChecksumCompletion::Completed);
    let saves = || {
        sent.borrow()
            .iter()
            .filter_map(|command| match command {
                Command::SaveChecksumManifest {
                    save_id, request, ..
                } => Some((*save_id, request.overwrite, request.destination.clone())),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let chosen = std::env::temp_dir().join("chosen.sha256");
    let s = p.checksums.sessions.get_mut(&id).unwrap();
    s.suggested = chosen.clone();
    s.save_pending = Some(ChecksumAlgorithm::Sha256);
    p.begin_pending_checksum_saves(&send);
    let first = saves();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].2, chosen);
    assert!(!first[0].1);
    let generation = p.checksums.sessions[&id].generation;
    p.apply(Event::ChecksumManifest(Box::new(ChecksumManifestEvent {
        save_id: first[0].0,
        job: id,
        generation,
        update: ChecksumManifestUpdate::Finished(Err(FsError {
            operation: "save checksum manifest",
            path: chosen.clone(),
            message: "Manifest destination already exists.".into(),
            kind: std::io::ErrorKind::AlreadyExists,
        })),
    })));
    let s = &p.checksums.sessions[&id];
    assert!(s.error.is_empty(), "an existing file is a question, not an error");
    assert!(s.save_prompt.as_ref().is_some_and(|prompt| prompt.overwrite
        && prompt.message.contains("chosen.sha256")));
    p.dismiss_checksum_save_prompt(id);
    assert!(p.checksums.sessions[&id].save_prompt.is_none());
    assert_eq!(saves().len(), 1, "Cancel writes nothing");
    p.checksums.sessions.get_mut(&id).unwrap().save_pending = Some(ChecksumAlgorithm::Sha256);
    p.begin_pending_checksum_saves(&send);
    let second = saves();
    p.apply(Event::ChecksumManifest(Box::new(ChecksumManifestEvent {
        save_id: second[1].0,
        job: id,
        generation,
        update: ChecksumManifestUpdate::Finished(Err(FsError {
            operation: "save checksum manifest",
            path: chosen.clone(),
            message: "Manifest destination already exists.".into(),
            kind: std::io::ErrorKind::AlreadyExists,
        })),
    })));
    p.confirm_checksum_save(id, &send);
    let all = saves();
    assert_eq!(all.len(), 3);
    assert!(all[2].1, "Replace sends the confirmed overwrite");
    assert_eq!(all[2].2, chosen);
}
