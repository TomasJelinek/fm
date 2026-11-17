//! A Rename or Delete that completed but left private staging behind still completed.
use super::*;
use std::io;

fn residue(cleanup_unconfirmed: bool, kind: io::ErrorKind) -> MutationResidue {
    let path = std::env::temp_dir()
        .join("midnight-operation-worker-fixture")
        .join(".midnight-mutation-residue");
    MutationResidue {
        error: FsError::from_io("mutate entry", &path, kind.into()),
        path,
        cleanup_unconfirmed,
    }
}

/// The terminal progress; these jobs must never ask for a Rename decision.
async fn terminal_without_decision(h: &mut Harness) -> Arc<OperationProgress> {
    loop {
        match h.next().await {
            OperationEvent::Progress(progress) if progress.state.is_terminal() => return progress,
            OperationEvent::RenameConflict(conflict) => panic!("unexpected prompt: {conflict:?}"),
            _ => {}
        }
    }
}

#[tokio::test]
async fn a_completed_rename_with_a_staging_residue_completes_with_errors_and_reveals_it() {
    // Neither the cleanup's native kind nor its confirmation may turn the completed
    // rename into a conflict prompt, a cancellation or a failure.
    for (kind, cleanup_unconfirmed) in [
        (io::ErrorKind::AlreadyExists, false),
        (io::ErrorKind::Interrupted, true),
    ] {
        let mut h = Harness::new(MutationKind::Rename);
        let residue = residue(cleanup_unconfirmed, kind);
        *h.calls.residue.lock().unwrap() = Some(residue.clone());
        h.controller.prepare(h.request.clone()).await;
        h.preview().await;
        h.controller
            .confirm(h.request.id, Some("renamed 文档.txt".into()))
            .await;
        let progress = terminal_without_decision(&mut h).await;
        let destination = h.request.source.path.join("renamed 文档.txt");
        assert_eq!(
            progress.state,
            OperationState::CompletedWithErrors,
            "{kind:?}"
        );
        assert_eq!(
            (
                progress.completed,
                progress.failed,
                progress.processed_entries
            ),
            (1, 1, 1)
        );
        assert_eq!(progress.renamed_to.as_ref(), Some(&destination));
        assert_eq!(h.calls.renamed.lock().unwrap().len(), 1);
        assert_eq!(progress.errors.len(), 1);
        let issue = &progress.errors[0];
        assert_eq!(issue.path, residue.path);
        assert_eq!(issue.kind, io::ErrorKind::Other);
        let completed = format!(
            "Renamed to {}, but its temporary staging folder",
            destination.display()
        );
        assert!(issue.message.starts_with(&completed), "{issue:?}");
        assert_eq!(
            issue.message.contains("remains at"),
            !cleanup_unconfirmed,
            "{issue:?}"
        );
        assert!(progress.message.contains(&issue.message));
    }
}

#[tokio::test]
async fn a_completed_delete_with_a_staging_residue_counts_its_target_as_completed() {
    for (mode, done) in [
        (DeleteMode::Trash, "Moved"),
        (DeleteMode::Permanent, "Deleted"),
    ] {
        let mut h = Harness::new(MutationKind::Delete(mode));
        let residue = residue(false, io::ErrorKind::DirectoryNotEmpty);
        *h.calls.residue.lock().unwrap() = Some(residue.clone());
        h.controller.prepare(h.request.clone()).await;
        h.preview().await;
        h.controller.confirm(h.request.id, None).await;
        let progress = h.finished().await;
        assert_eq!(
            progress.state,
            OperationState::CompletedWithErrors,
            "{mode:?}"
        );
        assert_eq!(
            (
                progress.completed,
                progress.failed,
                progress.processed_entries
            ),
            (1, 1, 1)
        );
        assert_eq!(progress.errors.len(), 1);
        let issue = &progress.errors[0];
        assert_eq!(
            (issue.path.as_path(), issue.kind),
            (residue.path.as_path(), io::ErrorKind::Other)
        );
        let target = h.request.source.entries[0].path.display();
        assert!(
            issue.message.starts_with(&format!("{done} {target}")),
            "{issue:?}"
        );
        assert!(
            progress
                .message
                .contains(&residue.path.display().to_string())
        );
        assert!(
            !progress.message.contains("have not been undone"),
            "{}",
            progress.message
        );
    }
}

/// Deletes every named entry of one directory directly on the worker.
fn delete_selection(names: &[&str], calls: Arc<Calls>) -> OperationProgress {
    let root = std::env::temp_dir().join("midnight-residue-fixture");
    let entries: Arc<[Entry]> = names
        .iter()
        .enumerate()
        .map(|(index, name)| Entry::new(EntryId(index as u64), root.join(name), EntryKind::File))
        .collect();
    let view = Arc::new(DirectoryIndex::from_entries(&entries));
    let selection = Arc::new(
        SelectionSnapshot::from_bits(view.clone(), vec![(1u64 << entries.len()) - 1]).unwrap(),
    );
    let request = PrepareOperation {
        id: OperationId(9),
        kind: MutationKind::Delete(DeleteMode::Permanent),
        source: VisibleDirectory {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            path: root,
            entries,
        },
        view,
        selection,
        focused: None,
    };
    let mut job = worker::prepare(
        request.clone(),
        Some(Arc::new(FakeService(calls))),
        Cancellation::default(),
        Arc::new(Mutex::new(worker::initial(&request))),
        Arc::new(Semaphore::new(MAX_METADATA_TARGETS)),
    )
    .unwrap();
    job.confirm(None).unwrap();
    job.set_state(OperationState::Running);
    let (sender, _events) = mpsc::channel(8);
    job.execute(Outbox {
        sender,
        wakeup: Arc::new(|| {}),
    })
}

#[test]
fn a_residue_of_one_target_never_hides_that_every_target_completed() {
    let calls = Arc::new(Calls::default());
    *calls.residue.lock().unwrap() = Some(residue(true, io::ErrorKind::NotFound));
    let progress = delete_selection(&["first", "second"], calls.clone());
    assert_eq!(progress.state, OperationState::CompletedWithErrors);
    assert_eq!(
        (progress.total, progress.completed, progress.failed),
        (2, 2, 1)
    );
    assert_eq!(calls.deleted.lock().unwrap().len(), 2);
    assert_eq!(progress.errors.len(), 1);
    let first = format!(
        "Deleted {}, but",
        progress.source_directory.join("first").display()
    );
    assert!(
        progress.errors[0].message.starts_with(&first),
        "{:?}",
        progress.errors
    );
    assert!(!progress.message.contains("have not been undone"));
}

#[tokio::test]
async fn a_rename_that_stranded_its_entry_fails_without_a_prompt_and_keeps_the_location() {
    // Linux reports a failed restoration as kind Other: the preserved location must
    // stay the job's error instead of being dropped by the name-conflict flow.
    let mut h = Harness::new(MutationKind::Rename);
    let preserved = "remaining entry preserved at /data/.midnight-mutation-1-2-3/item";
    *h.calls.rename_failure.lock().unwrap() = Some(FsError::from_io(
        "mutate entry",
        &h.request.source.entries[0].path,
        io::Error::other(format!("File exists (os error 17); {preserved}")),
    ));
    h.controller.prepare(h.request.clone()).await;
    h.preview().await;
    h.controller
        .confirm(h.request.id, Some("occupied.txt".into()))
        .await;
    let progress = terminal_without_decision(&mut h).await;
    assert_eq!(progress.state, OperationState::Failed);
    assert_eq!((progress.completed, progress.failed), (0, 1));
    assert_eq!(progress.renamed_to, None);
    assert!(progress.errors[0].message.contains(preserved));
    assert!(progress.message.contains(preserved));
    assert_eq!(h.calls.renamed.lock().unwrap().len(), 1);
}
