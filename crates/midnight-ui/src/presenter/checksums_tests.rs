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

/// A presenter whose left panel lists these entries of the temporary folder.
fn listing(entries: &[(&str, EntryKind)]) -> (Presenter, PathBuf) {
    let root = std::env::temp_dir();
    let mut p = Presenter::new(root.clone(), root.clone());
    let entries: Arc<[Entry]> = entries
        .iter()
        .enumerate()
        .map(|(index, (name, kind))| Entry::new(EntryId(index as u64 + 1), root.join(name), *kind))
        .collect();
    let pane = &mut p.panes[0];
    pane.entries = entries.clone();
    pane.index = Arc::new(DirectoryIndex::from_entries(&entries));
    (p, root)
}

/// Captured with nothing selected: only the focused row, `path`.
fn focused(p: &Presenter, path: PathBuf) -> SelectedFiles {
    SelectedFiles {
        selection: Arc::new(SelectionSnapshot::default()),
        view: p.panes[0].index.clone(),
        fallback: Some(path),
    }
}

#[test]
fn focused_checksum_sidecar_prefills_verification_without_hashing_the_manifest() {
    let (mut p, root) = listing(&[("日本語.MD5", EntryKind::File)]);
    let manifest = root.join("日本語.MD5");
    let commands = std::cell::RefCell::new(Vec::new());
    let files = focused(&p, manifest.clone());
    let id = p
        .open_captured_checksums(PaneId::Left, root, files, &|command| {
            commands.borrow_mut().push(command);
            true
        })
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
    finish_reporting(p, id, completion, Vec::new(), Vec::new());
}

/// [`finish`] with the backend's diagnostics and the limits the run reached.
fn finish_reporting(
    p: &mut Presenter,
    id: ChecksumJobId,
    completion: ChecksumCompletion,
    diagnostics: Vec<FsError>,
    reached_budgets: Vec<ChecksumBudget>,
) {
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
                reached_budgets,
                diagnostics,
            },
            retention,
        })),
    })));
}

#[test]
fn a_verification_refused_for_mixed_algorithms_names_that_reason() {
    let send = |_| true;
    let (mut p, id) = setup(&send);
    finish(&mut p, id, ChecksumCompletion::Completed);
    assert!(p.start_checksum_verification(id, "mixed.sha256", &send));
    let s = &p.checksums.sessions[&id];
    let (generation, manifest) = (s.generation, s.directory.join("mixed.sha256"));
    // What the engine reports for a SHA-256 line followed by an MD5 line.
    let refused = FsError::from_io(
        "verify checksums",
        &manifest,
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Checksum manifest mixes different algorithms.",
        ),
    );
    p.apply(Event::Checksum(Box::new(ChecksumEvent {
        job: id,
        generation,
        update: ChecksumUpdate::Finished(Arc::new(ChecksumRun {
            job: id,
            generation,
            algorithms: ChecksumAlgorithms::default(),
            captured_roots: vec![manifest].into(),
            rows: Arc::from([]),
            summary: ChecksumSummary {
                completion: ChecksumCompletion::Failed,
                progress: ChecksumProgress {
                    errors: 1,
                    ..Default::default()
                },
                reached_budgets: vec![],
                diagnostics: vec![refused],
            },
            retention: ChecksumRetention::new(()),
        })),
    })));
    let s = &p.checksums.sessions[&id];
    assert!(!s.running);
    assert_eq!(
        s.issue().as_deref(),
        Some("Checksum manifest mixes different algorithms.")
    );
    let details = s.details(&NumberLocale::default());
    assert!(details.contains("mixes different algorithms"), "{details}");
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
fn manifest_filename_uses_captured_folder_and_gets_the_format_extension() {
    let (mut p, id) = setup(&|_| true);
    finish(&mut p, id, ChecksumCompletion::Completed);
    // The chosen name has no extension; the captured folder stays after navigation.
    let chosen = p.checksums.sessions[&id]
        .suggested
        .with_file_name("export 日本語");
    p.checksums.sessions.get_mut(&id).unwrap().suggested = chosen.clone();
    p.panes[0].path.push("navigated-away");
    let commands = std::cell::RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    p.begin_checksum_save(id, ChecksumAlgorithm::Sha256, &send);
    let sent = commands.borrow();
    assert_eq!(sent.len(), 1);
    let Command::SaveChecksumManifest { request, .. } = &sent[0] else {
        panic!("Only the chosen destination may enter the manifest queue");
    };
    assert_eq!(request.destination, chosen.with_extension("sha256"));
    assert!(p.checksums.sessions[&id].error.is_empty());
}

#[test]
fn partial_checksum_save_requires_exact_current_run_acknowledgement() {
    let (mut p, id) = setup(&|_| true);
    finish(&mut p, id, ChecksumCompletion::Incomplete);
    let path = std::env::temp_dir().join("checksums.sha256");
    p.checksums.sessions.get_mut(&id).unwrap().suggested = path.clone();
    let commands = std::cell::RefCell::new(Vec::new());
    let send = |c| {
        commands.borrow_mut().push(c);
        true
    };
    p.begin_checksum_save(id, ChecksumAlgorithm::Sha256, &send);
    assert!(
        commands.borrow().is_empty(),
        "partial results are asked first"
    );
    p.confirm_checksum_save(id, &send);
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
fn ctrl_f8_without_a_selection_calculates_the_focused_folder_or_the_shown_folder() {
    let root = std::env::temp_dir();
    let mut p = Presenter::new(root.clone(), root.clone());
    let folder = root.join("fotky 文档");
    let entries: Arc<[Entry]> = vec![
        Entry::new(EntryId(1), folder.clone(), EntryKind::Directory),
        Entry::new(EntryId(2), root.join("photo.jpg"), EntryKind::File),
    ]
    .into();
    let pane = &mut p.panes[0];
    pane.entries = entries.clone();
    pane.index = Arc::new(DirectoryIndex::from_entries(&entries));
    // The focused folder is the target, on the Calculate page.
    pane.parent_cursor = false;
    pane.cursor = Some(EntryId(1));
    p.request_checksums(PaneId::Left, &|_| true);
    let (&focused, s) = p.checksums.sessions.iter().next().unwrap();
    assert_eq!(s.files.fallback.as_ref(), Some(&folder));
    assert!(!s.verify_initial && !s.running);
    assert_eq!(s.algorithms, ChecksumAlgorithms::default());
    // On the parent row with nothing selected: the shown folder's files.
    let pane = &mut p.panes[0];
    pane.parent_cursor = true;
    pane.cursor = None;
    p.request_checksums(PaneId::Left, &|_| true);
    let s = p
        .checksums
        .sessions
        .iter()
        .find_map(|(id, s)| (*id != focused).then_some(s))
        .unwrap();
    assert_eq!(s.files.fallback.as_deref(), Some(p.panes[0].display_path()));
    assert!(!s.verify_initial);
}

#[test]
fn opening_calculates_nothing_and_a_selected_checksum_file_opens_verification() {
    let (mut p, root) = listing(&[
        ("photo.jpg", EntryKind::File),
        ("photos.SHA256", EntryKind::File),
    ]);
    let sent = std::cell::RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    let files = |p: &Presenter, name: &str| focused(p, root.join(name));
    let photo = files(&p, "photo.jpg");
    let plain = p
        .open_captured_checksums(PaneId::Left, root.clone(), photo, &send)
        .unwrap();
    assert!(!p.checksums.sessions[&plain].running);
    assert!(!p.checksums.sessions[&plain].verify_initial);
    let sums = files(&p, "photos.SHA256");
    let manifest = p
        .open_captured_checksums(PaneId::Left, root.clone(), sums, &send)
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
    assert!(
        s.error.is_empty(),
        "an existing file is a question, not an error"
    );
    assert!(
        s.save_prompt
            .as_ref()
            .is_some_and(|prompt| prompt.overwrite && prompt.message.contains("chosen.sha256"))
    );
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

/// Native folders whose names are not valid Unicode; the destination tests share them.
#[cfg(unix)]
#[path = "checksums/destination_unix_tests.rs"]
pub(super) mod raw_paths;
#[cfg(windows)]
#[path = "checksums/destination_windows_tests.rs"]
pub(super) mod raw_paths;

/// Every manifest save sent: its ID and request.
fn sent_saves(sent: &[Command]) -> Vec<(ChecksumManifestId, ChecksumManifestRequest)> {
    sent.iter()
        .filter_map(|command| match command {
            Command::SaveChecksumManifest {
                save_id, request, ..
            } => Some((*save_id, (**request).clone())),
            _ => None,
        })
        .collect()
}

fn already_exists(p: &mut Presenter, id: ChecksumJobId, save_id: ChecksumManifestId) {
    let generation = p.checksums.sessions[&id].generation;
    p.apply(Event::ChecksumManifest(Box::new(ChecksumManifestEvent {
        save_id,
        job: id,
        generation,
        update: ChecksumManifestUpdate::Finished(Err(FsError::from_io(
            "save checksum manifest",
            Path::new("existing"),
            std::io::ErrorKind::AlreadyExists.into(),
        ))),
    })));
}

#[test]
fn a_new_run_discards_the_save_question_about_the_previous_one() {
    let sent = std::cell::RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    let (mut p, id) = setup(&send);
    finish(&mut p, id, ChecksumCompletion::Incomplete);
    p.begin_checksum_save(id, ChecksumAlgorithm::Sha256, &send);
    assert!(
        p.checksums.sessions[&id].save_prompt.is_some(),
        "partial results are asked about first"
    );
    let algorithms = p.checksums.sessions[&id].algorithms;
    assert!(p.start_checksums(id, algorithms, false, &send));
    assert!(p.checksums.sessions[&id].save_prompt.is_none());
    sent.borrow_mut().clear();
    p.confirm_checksum_save(id, &send);
    assert!(
        sent_saves(&sent.borrow()).is_empty(),
        "an answer about old results saves nothing of the new run"
    );
}

#[test]
fn a_partial_save_sends_the_counts_its_question_showed_and_a_refused_answer_is_kept() {
    let sent = std::cell::RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    let (mut p, id) = setup(&send);
    finish(&mut p, id, ChecksumCompletion::Incomplete);
    p.begin_checksum_save(id, ChecksumAlgorithm::Sha256, &send);
    let shown = p.checksums.sessions[&id]
        .save_prompt
        .as_ref()
        .and_then(|prompt| prompt.acknowledged)
        .expect("the question carries the counts it shows");
    // An answer while other work runs is refused aloud, and the question stays.
    p.checksums.sessions.get_mut(&id).unwrap().running = true;
    p.confirm_checksum_save(id, &send);
    let s = &p.checksums.sessions[&id];
    assert!(sent_saves(&sent.borrow()).is_empty());
    assert!(!s.error.is_empty(), "a refused answer is reported");
    assert!(
        s.save_prompt.is_some(),
        "the question can still be answered"
    );
    p.checksums.sessions.get_mut(&id).unwrap().running = false;
    p.confirm_checksum_save(id, &send);
    let saves = sent_saves(&sent.borrow());
    let [(_, request)] = saves.as_slice() else {
        panic!("one save: {saves:?}");
    };
    assert_eq!(request.acknowledge_partial, Some(shown));
    assert!(p.checksums.sessions[&id].save_prompt.is_none());
}

#[test]
fn replacing_keeps_the_exact_native_destination_it_asked_about() {
    let sent = std::cell::RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    let (mut p, id) = setup(&send);
    finish(&mut p, id, ChecksumCompletion::Completed);
    // A folder name that is not valid Unicode, and an extension the save appends.
    let chosen = raw_paths::raw_parent().join("checksums");
    p.checksums.sessions.get_mut(&id).unwrap().suggested = chosen.clone();
    p.begin_checksum_save(id, ChecksumAlgorithm::Sha256, &send);
    let expected = chosen.with_extension("sha256");
    let first = sent_saves(&sent.borrow());
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].1.destination, expected);
    already_exists(&mut p, id, first[0].0);
    p.confirm_checksum_save(id, &send);
    let saves = sent_saves(&sent.borrow());
    assert_eq!(saves.len(), 2, "{:?}", p.checksums.sessions[&id].error);
    assert!(saves[1].1.overwrite);
    assert_eq!(saves[1].1.destination, expected);
}

#[test]
fn the_window_text_names_a_refused_input_the_limits_and_a_kept_replaced_file() {
    let sent = std::cell::RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    let (mut p, id) = setup(&send);
    let refused = FsError::from_io(
        "verify checksums",
        Path::new("/data/sums.sha256"),
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Manifest entries must be relative names below the manifest's folder.",
        ),
    );
    finish_reporting(
        &mut p,
        id,
        ChecksumCompletion::Failed,
        vec![refused],
        vec![ChecksumBudget::Rows],
    );
    let locale = NumberLocale::default();
    let s = &p.checksums.sessions[&id];
    assert_eq!(
        s.issue().as_deref(),
        Some("Manifest entries must be relative names below the manifest's folder.")
    );
    let details = s.details(&locale);
    assert!(details.contains("must be relative names"), "{details}");
    assert!(
        details.contains("Reached resource limits: result rows"),
        "{details}"
    );
    // A completed run saved over an existing file reports where that file is kept.
    let algorithms = p.checksums.sessions[&id].algorithms;
    assert!(p.start_checksums(id, algorithms, false, &send));
    finish(&mut p, id, ChecksumCompletion::Completed);
    let saved = std::env::temp_dir().join("checksums.sha256");
    let kept = std::env::temp_dir()
        .join(".midnight-recovery-1")
        .join("item");
    p.checksums.sessions.get_mut(&id).unwrap().suggested = saved.clone();
    p.begin_checksum_save(id, ChecksumAlgorithm::Sha256, &send);
    let (save_id, _) = sent_saves(&sent.borrow())[0].clone();
    let generation = p.checksums.sessions[&id].generation;
    p.apply(Event::ChecksumManifest(Box::new(ChecksumManifestEvent {
        save_id,
        job: id,
        generation,
        update: ChecksumManifestUpdate::Finished(Ok(ChecksumManifestReceipt {
            path: saved.clone(),
            included: 1,
            excluded: 0,
            verified_bytes: 77,
            recovery: vec![kept.clone()],
        })),
    })));
    let s = &p.checksums.sessions[&id];
    assert_eq!(
        s.save_status(&locale),
        "Saved checksums.sha256 · 1 included · the replaced file is kept for recovery"
    );
    let details = s.details(&locale);
    assert!(details.contains(saved.to_str().unwrap()), "{details}");
    assert!(details.contains(kept.to_str().unwrap()), "{details}");
    assert!(details.contains("Delete it when you no longer need it."));
    assert!(s.issue().is_none());
}

#[test]
fn a_selected_checksum_file_opens_verification_from_the_selection_itself() {
    let root = std::env::temp_dir();
    let mut p = Presenter::new(root.clone(), root.clone());
    let entries: Arc<[Entry]> = vec![
        Entry::new(EntryId(1), root.join("photo.jpg"), EntryKind::File),
        Entry::new(EntryId(2), root.join("photos.sha256"), EntryKind::File),
    ]
    .into();
    let pane = &mut p.panes[0];
    pane.entries = entries.clone();
    pane.index = Arc::new(DirectoryIndex::from_entries(&entries));
    // Only the manifest is selected; the cursor stays on the photo.
    let files = SelectedFiles {
        selection: Arc::new(SelectionSnapshot::from_bits(pane.index.clone(), vec![0b10]).unwrap()),
        view: pane.index.clone(),
        fallback: Some(root.join("photo.jpg")),
    };
    let id = p
        .open_captured_checksums(PaneId::Left, root.clone(), files, &|_| true)
        .unwrap();
    let s = &p.checksums.sessions[&id];
    assert!(s.verify_initial);
    assert_eq!(s.manifest_suggested, root.join("photos.sha256"));
}

#[test]
fn listed_files_wait_for_their_rows_in_reading_order_and_leave_with_the_run() {
    let (mut p, id) = setup(&|_| true);
    let generation = p.checksums.sessions[&id].generation;
    let root = std::env::temp_dir();
    let event = |update| {
        Event::Checksum(Box::new(ChecksumEvent {
            job: id,
            generation,
            update,
        }))
    };
    let listed: Arc<[ChecksumPlannedFile]> = ["a", "b 文档", "c"]
        .iter()
        .map(|name| ChecksumPlannedFile {
            path: root.join(name),
            size: Some(3),
        })
        .collect();
    p.apply(event(ChecksumUpdate::Planned(listed.clone())));
    let pending = |p: &Presenter| -> Vec<PathBuf> {
        p.checksums.sessions[&id]
            .pending()
            .map(|file| file.path.clone())
            .collect()
    };
    assert_eq!(
        pending(&p),
        [root.join("a"), root.join("b 文档"), root.join("c")]
    );
    // A listed file's row takes its place; the others keep waiting in order. A row
    // for an entry that was not listed (an unreadable folder) waits for nothing.
    let row = |path: PathBuf, id: u64| {
        Arc::new(ChecksumRow {
            id: ChecksumRowId(id),
            path,
            identity: None,
            kind: Some(EntryKind::File),
            state: ChecksumRowState::Calculated,
            bytes_read: 3,
            digests: Vec::new(),
            error: None,
            retention: ChecksumRetention::new(()),
        })
    };
    let rows: Arc<[_]> = vec![row(root.join("b 文档"), 1), row(root.join("other"), 2)].into();
    p.apply(event(ChecksumUpdate::Batch(rows)));
    assert_eq!(p.checksums.sessions[&id].rows().len(), 2);
    assert_eq!(pending(&p), [root.join("a"), root.join("c")]);
    assert_eq!(p.checksums.sessions[&id].pending_count(), 2);
    let at = |p: &Presenter, k| {
        p.checksums.sessions[&id]
            .pending_at(k)
            .map(|file| file.path.clone())
    };
    assert_eq!(at(&p, 1), Some(root.join("c")), "past a file read early");
    // Back in listing order: a's row makes c the next file to read.
    p.apply(event(ChecksumUpdate::Batch(
        vec![row(root.join("a"), 3)].into(),
    )));
    assert_eq!(at(&p, 0), Some(root.join("c")));
    assert_eq!(at(&p, 1), None);
    // Only results remain once the run finishes.
    finish(&mut p, id, ChecksumCompletion::Cancelled);
    assert_eq!(p.checksums.sessions[&id].pending_count(), 0);
}

#[test]
fn copy_writes_the_lines_a_saved_checksum_file_has() {
    let commands = std::cell::RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    let (mut p, id) = setup(&send);
    finish(&mut p, id, ChecksumCompletion::Completed);
    commands.borrow_mut().clear();
    p.copy_checksum_file(id, ChecksumAlgorithm::Sha256, &send);
    let copied = |commands: &[Command]| {
        commands.iter().find_map(|command| match command {
            Command::CopyText { text, .. } => Some(text.clone()),
            _ => None,
        })
    };
    // As Save writes it to the suggested folder: "<hash> *<name relative to it>".
    assert_eq!(
        copied(&commands.borrow()),
        Some(format!("{} *Žluťoučký 日本語.txt\n", "42".repeat(32)))
    );
    assert!(p.checksums.sessions[&id].copy_note.is_none());
    // A file that is not below the checksum file's folder is left out, and a note
    // says so. It is not an error: an earlier, unrelated error stays as it was.
    commands.borrow_mut().clear();
    let s = p.checksums.sessions.get_mut(&id).unwrap();
    let suggested = s.suggested.clone();
    s.suggested = s.directory.join("jinde").join("checksums.sha256");
    let unrelated = "File chooser queue is busy. Try Browse again.";
    s.error = unrelated.into();
    p.copy_checksum_file(id, ChecksumAlgorithm::Sha256, &send);
    assert_eq!(copied(&commands.borrow()), None);
    let locale = NumberLocale::default();
    let s = &p.checksums.sessions[&id];
    assert_eq!(s.error, unrelated);
    assert_eq!(
        s.save_status(&locale),
        "Nothing copied. Left out: 1 file not below the checksum file's folder."
    );
    // A copy that leaves nothing out drops the note, and still not the error.
    p.checksums.sessions.get_mut(&id).unwrap().suggested = suggested;
    p.copy_checksum_file(id, ChecksumAlgorithm::Sha256, &send);
    assert!(copied(&commands.borrow()).is_some());
    let s = &p.checksums.sessions[&id];
    assert!(s.copy_note.is_none());
    assert_eq!(s.error, unrelated);
    // An algorithm that was not calculated copies nothing.
    commands.borrow_mut().clear();
    p.copy_checksum_file(id, ChecksumAlgorithm::Md5, &send);
    assert_eq!(copied(&commands.borrow()), None);
}

#[test]
fn copy_says_how_many_files_it_left_out_for_each_reason() {
    let commands = std::cell::RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    let (mut p, id) = setup(&send);
    // The checksum file goes to `qa`; a folder name below it is not valid Unicode.
    let qa = raw_paths::plain_parent().parent().unwrap().to_owned();
    let elsewhere = std::env::temp_dir();
    let paths = [
        qa.join("ok.txt"),
        qa.join("#notes.txt"),
        raw_paths::raw_parent().join("x.txt"),
        elsewhere.join("first.txt"),
        qa.join(" leading space.txt"),
        elsewhere.join("second.txt"),
    ];
    let generation = p.checksums.sessions[&id].generation;
    let retention = ChecksumRetention::new(());
    let rows: Vec<_> = paths
        .iter()
        .enumerate()
        .map(|(index, path)| {
            Arc::new(ChecksumRow {
                id: ChecksumRowId(index as u64 + 1),
                path: path.clone(),
                identity: None,
                kind: Some(EntryKind::File),
                state: ChecksumRowState::Calculated,
                bytes_read: 3,
                digests: vec![ChecksumDigest {
                    algorithm: ChecksumAlgorithm::Crc32,
                    bytes: vec![0x42; 4],
                }],
                error: None,
                retention: retention.clone(),
            })
        })
        .collect();
    p.apply(Event::Checksum(Box::new(ChecksumEvent {
        job: id,
        generation,
        update: ChecksumUpdate::Finished(Arc::new(ChecksumRun {
            job: id,
            generation,
            algorithms: ChecksumAlgorithms::new(&[ChecksumAlgorithm::Crc32]).unwrap(),
            captured_roots: vec![qa.clone()].into(),
            rows: rows.into(),
            summary: ChecksumSummary {
                completion: ChecksumCompletion::Completed,
                progress: ChecksumProgress {
                    discovered: 6,
                    calculated: 6,
                    ..Default::default()
                },
                reached_budgets: vec![],
                diagnostics: vec![],
            },
            retention,
        })),
    })));
    p.checksums.sessions.get_mut(&id).unwrap().suggested = qa.join("checksums.sfv");
    commands.borrow_mut().clear();
    p.copy_checksum_file(id, ChecksumAlgorithm::Crc32, &send);
    let copied: Vec<_> = commands
        .borrow()
        .iter()
        .filter_map(|command| match command {
            Command::CopyText { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(copied, ["ok.txt  42424242\n"]);
    let locale = NumberLocale::default();
    let s = &p.checksums.sessions[&id];
    let note = "Copied 1 checksum. Left out: 2 files whose names SFV cannot hold \
                unambiguously; 1 file whose name is not valid Unicode; 2 files not below \
                the checksum file's folder.";
    assert_eq!(s.save_status(&locale), note);
    assert!(s.details(&locale).contains(note), "{}", s.details(&locale));
    // A note, not an error: no issue line, no attention.
    assert!(s.error.is_empty());
    assert_eq!(s.issue(), None);
}

#[test]
fn details_count_one_error_in_the_singular() {
    let (mut p, id) = setup(&|_| true);
    finish(&mut p, id, ChecksumCompletion::CompletedWithErrors);
    p.checksums.sessions.get_mut(&id).unwrap().progress.errors = 1;
    let details = p.checksums.sessions[&id].details(&NumberLocale::default());
    assert!(
        details.contains("1 found · 0 skipped · 1 error"),
        "{details}"
    );
    assert!(!details.contains("1 errors"), "{details}");
}

#[test]
fn a_folder_named_like_a_checksum_file_is_calculated_and_never_verified() {
    let (mut p, root) = listing(&[
        ("photos.sfv", EntryKind::Directory),
        ("sums.sha256", EntryKind::File),
    ]);
    let send = |_| true;
    let opened = |p: &mut Presenter, files: SelectedFiles| {
        let id = p
            .open_captured_checksums(PaneId::Left, root.clone(), files, &send)
            .unwrap();
        let s = &p.checksums.sessions[&id];
        let shown = (s.verify_initial, s.manifest_suggested.clone());
        assert!(p.close_checksums(id, &send));
        shown
    };
    // The focused folder is calculated; a focused checksum file is verified.
    let folder = focused(&p, root.join("photos.sfv"));
    assert!(!opened(&mut p, folder).0, "a focused folder");
    let file = focused(&p, root.join("sums.sha256"));
    assert_eq!(opened(&mut p, file), (true, root.join("sums.sha256")));
    // A selected folder named like one is no checksum file either.
    let index = p.panes[0].index.clone();
    let selected = SelectedFiles {
        selection: Arc::new(SelectionSnapshot::from_bits(index.clone(), vec![0b01]).unwrap()),
        view: index,
        fallback: None,
    };
    assert!(!opened(&mut p, selected).0, "a selected folder");
    // On the parent row inside such a folder, Ctrl+F8 calculates that folder.
    let pane = &mut p.panes[0];
    pane.path = root.join("archive.md5");
    pane.parent_cursor = true;
    pane.cursor = None;
    p.request_checksums(PaneId::Left, &send);
    let s = p.checksums.sessions.values().next().unwrap();
    assert!(!s.verify_initial, "the shown folder");
    assert_eq!(s.files.fallback, Some(root.join("archive.md5")));
}

#[test]
fn ctrl_f8_on_the_parent_row_right_after_a_deselect_calculates_the_shown_folder() {
    let (mut p, _) = listing(&[("photo.jpg", EntryKind::File)]);
    let pane = &mut p.panes[0];
    pane.parent_cursor = true;
    pane.cursor = None;
    // A deselect is still being applied when Ctrl+F8 arrives.
    pane.selection_revision = SelectionRevision(1);
    let send = |_| true;
    p.request_checksums(PaneId::Left, &send);
    assert!(
        p.checksums.sessions.is_empty(),
        "it waits for the selection"
    );
    let navigation = NavigationId(p.panes[0].generation);
    p.apply_with_sender(
        Event::SelectionChanged {
            pane: PaneId::Left,
            navigation,
            revision: SelectionRevision(1),
            selection: Arc::new(SelectionSnapshot::default()),
            deferred: Default::default(),
        },
        &send,
    );
    let s = p
        .checksums
        .sessions
        .values()
        .next()
        .expect("the window opens once the selection is known");
    // On Calculate with the shown folder, not on an empty Verify page.
    assert!(!s.verify_initial);
    assert_eq!(s.files.fallback.as_deref(), Some(p.panes[0].display_path()));
}

#[test]
fn a_verification_starts_without_the_listed_files_of_an_earlier_calculation() {
    let (mut p, id) = setup(&|_| true);
    let generation = p.checksums.sessions[&id].generation;
    let listed: Arc<[ChecksumPlannedFile]> = ["a", "b"]
        .iter()
        .map(|name| ChecksumPlannedFile {
            path: std::env::temp_dir().join(name),
            size: Some(3),
        })
        .collect();
    p.apply(Event::Checksum(Box::new(ChecksumEvent {
        job: id,
        generation,
        update: ChecksumUpdate::Planned(listed),
    })));
    // The calculation is refused after listing its files; they stay listed.
    p.apply(Event::ChecksumRejected(Box::new(ChecksumRejected {
        job: id,
        generation,
        error: FsError::from_io(
            "checksums",
            Path::new(""),
            std::io::ErrorKind::OutOfMemory.into(),
        ),
    })));
    assert_eq!(p.checksums.sessions[&id].pending_count(), 2);
    assert!(p.start_checksum_verification(id, "sums.sha256", &|_| true));
    assert_eq!(p.checksums.sessions[&id].pending_count(), 0);
    assert!(p.checksums.sessions[&id].rows().is_empty());
}

#[test]
fn verifying_again_checks_the_same_checksum_file_by_its_exact_path() {
    let sent = std::cell::RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    let (mut p, id) = setup(&send);
    finish(&mut p, id, ChecksumCompletion::Completed);
    assert!(
        !p.repeat_checksum_verification(id, &send),
        "calculated results have no checksum file to verify again"
    );
    assert!(p.start_checksum_verification(id, "日本語.sha256", &send));
    let verified = || {
        sent.borrow()
            .iter()
            .filter_map(|command| match command {
                Command::VerifyChecksumManifest(request) => {
                    Some((request.manifest.clone(), request.generation))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let manifest = verified()[0].0.clone();
    finish(&mut p, id, ChecksumCompletion::CompletedWithErrors);
    // The Verify page's field may name another file by now.
    p.checksums
        .sessions
        .get_mut(&id)
        .unwrap()
        .manifest_suggested = std::env::temp_dir().join("other.md5");
    assert!(p.repeat_checksum_verification(id, &send));
    assert_eq!(
        verified(),
        [
            (manifest.clone(), ChecksumGeneration(2)),
            (manifest.clone(), ChecksumGeneration(3))
        ]
    );
    let s = &p.checksums.sessions[&id];
    assert_eq!(s.verification, Some(manifest));
    assert!(s.running);
    assert!(
        !sent
            .borrow()
            .iter()
            .skip(1)
            .any(|command| matches!(command, Command::StartSelectedChecksums(_))),
        "no calculation replaces the verification"
    );
    // Not while it runs.
    assert!(!p.repeat_checksum_verification(id, &send));
}
