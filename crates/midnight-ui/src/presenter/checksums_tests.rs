use super::*;

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
    (p, id)
}

fn finish(p: &mut Presenter, id: ChecksumJobId, completion: ChecksumCompletion) {
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
    assert!(p.close_checksums(id, &|_| true));
    assert!(!p.checksums.sessions.contains_key(&id));
    assert!(!p.take_retired().is_empty());
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
        acknowledge: false,
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
        acknowledge,
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
    p.apply(Event::Checksum(Box::new(ChecksumEvent {
        job: id,
        generation: ChecksumGeneration(1),
        update: ChecksumUpdate::Started,
    })));
    assert!(p.checksums.sessions.is_empty());
    assert!(!p.take_retired().is_empty());
}
