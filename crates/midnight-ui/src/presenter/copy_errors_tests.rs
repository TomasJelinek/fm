use super::*;
use std::{cell::RefCell, io};

fn presenter() -> Presenter {
    let mut p = Presenter::new("source".into(), "destination".into());
    p.start_copy(
        FileClipboard {
            paths: vec!["source/資料.txt".into()],
            intent: FileClipboardIntent::Copy,
        },
        "destination".into(),
        &|_| true,
    );
    p
}

fn failure(p: &mut Presenter, version: u64, can_retry: bool, can_skip: bool) {
    p.apply(Event::Copy(CopyEvent::Failure(Arc::new(CopyFailure {
        id: p.copies.progress[0].id,
        version,
        error: FsError::from_io(
            "write copy",
            Path::new("destination/資料.txt"),
            io::Error::from(io::ErrorKind::PermissionDenied),
        ),
        source: Some("source/資料.txt".into()),
        destination: "destination/資料.txt".into(),
        stage: CopyErrorStage::Writing,
        can_retry,
        can_skip,
    }))));
}

#[test]
fn failure_decisions_enforce_capabilities_and_preserve_exact_token_on_queue_refusal() {
    for (choice, retry, skip, accepted) in [
        (8, true, false, true),
        (8, false, true, false),
        (0, false, true, true),
        (0, true, false, false),
        (9, false, true, true),
        (9, true, false, false),
        (3, false, false, true),
        (1, true, true, false),
        (4, true, true, false),
    ] {
        let mut p = presenter();
        failure(&mut p, 7, retry, skip);
        let token = "copy-error:1:7";
        p.decide_copy(token, choice, String::new(), &|_| false);
        assert_eq!(
            p.copies.failures.len(),
            1,
            "refusal keeps the decision live"
        );
        let sent = RefCell::new(Vec::new());
        p.decide_copy(token, choice, String::new(), &|command| {
            sent.borrow_mut().push(command);
            true
        });
        assert_eq!(sent.borrow().len(), usize::from(accepted));
        if accepted {
            let sent = sent.borrow();
            let Command::DecideCopy(decision) = &sent[0] else {
                panic!("expected decision")
            };
            assert_eq!(decision.id, OperationId(1));
            assert_eq!(decision.version, 7);
            assert!(matches!(
                (&decision.choice, choice),
                (CopyChoice::Retry, 8)
                    | (CopyChoice::Skip, 0)
                    | (CopyChoice::SkipAllErrors, 9)
                    | (CopyChoice::Cancel, 3)
            ));
        }
    }
}

#[test]
fn failure_replay_and_stale_events_cannot_revive_answered_or_terminal_decisions() {
    let mut p = presenter();
    failure(&mut p, 7, true, true);
    p.decide_copy("copy-error:1:6", 8, String::new(), &|_| {
        panic!("stale token")
    });
    p.decide_copy("copy-error:1:7", 8, String::new(), &|_| true);
    failure(&mut p, 7, true, true);
    assert!(p.copies.failures.is_empty());
    failure(&mut p, 8, true, true);
    failure(&mut p, 6, true, true);
    assert_eq!(p.copies.failures[0].version, 8);
    let mut done = (*p.copies.progress[0]).clone();
    done.state = CopyOperationState::Cancelled;
    p.apply(Event::Copy(CopyEvent::Progress(Arc::new(done))));
    failure(&mut p, 9, true, true);
    p.decide_copy("copy-error:1:8", 8, String::new(), &|_| {
        panic!("terminal token")
    });
    assert!(p.copies.failures.is_empty());
}

#[test]
fn deferred_failure_remains_in_jobs_and_resolve_restores_the_same_decision() {
    let mut p = presenter();
    failure(&mut p, 7, true, true);
    p.defer_copy_decision("copy-error:1:7");
    assert!(p.active_copy_decision().is_none());
    let row = crate::job_presentation::rows(&p).remove(0);
    assert!(row.attention && row.failed && row.cancellable);
    assert!(row.detail.contains("source/資料.txt"));
    assert!(row.detail.contains("destination/資料.txt"));
    p.resolve_copy_decision("copy-job:1");
    assert_eq!(p.active_copy_decision(), Some((OperationId(1), 7)));
}

#[test]
fn another_job_cannot_replace_a_visible_error_and_resolve_selects_the_requested_job() {
    let mut p = presenter();
    failure(&mut p, 7, true, true);
    p.start_copy(
        FileClipboard {
            paths: vec!["source/next".into()],
            intent: FileClipboardIntent::Copy,
        },
        "destination".into(),
        &|_| true,
    );
    let item = MutationItem {
        path: "destination/next".into(),
        kind: EntryKind::File,
        metadata: EntryMetadata::default(),
    };
    p.apply(Event::Copy(CopyEvent::Conflict(Arc::new(CopyConflict {
        id: OperationId(2),
        version: 1,
        source: item.clone(),
        target: item,
        destination: "destination/next".into(),
        can_replace: false,
        can_merge: false,
    }))));
    assert_eq!(p.active_copy_decision(), Some((OperationId(1), 7)));
    p.resolve_copy_decision("copy-job:2");
    assert_eq!(p.active_copy_decision(), Some((OperationId(2), 1)));
    p.decide_copy("copy:2:1", 0, String::new(), &|_| true);
    assert_eq!(p.active_copy_decision(), Some((OperationId(1), 7)));
}

#[test]
fn successful_transfer_keeps_terminal_history_without_a_completion_notice() {
    for intent in [TransferIntent::Copy, TransferIntent::Move] {
        let mut p = presenter();
        let mut done = (*p.copies.progress[0]).clone();
        done.intent = intent;
        done.state = CopyOperationState::Completed;
        done.message = "Saved and verified".into();
        p.apply(Event::Copy(CopyEvent::Progress(Arc::new(done))));
        assert!(p.desktop.notice.is_none());
        assert_eq!(p.copies.progress[0].state, CopyOperationState::Completed);
        assert!(
            crate::job_presentation::rows(&p)[0]
                .detail
                .contains("Saved and verified")
        );
    }
}

#[test]
fn accepted_cancellation_rejects_late_decisions_and_nonterminal_progress_until_terminal() {
    for cancel_from_error in [false, true] {
        let mut p = presenter();
        failure(&mut p, 7, true, true);
        if cancel_from_error {
            p.decide_copy("copy-error:1:7", 3, String::new(), &|_| true);
        } else {
            p.cancel_copy("copy-job:1", &|_| true);
        }
        assert_eq!(
            p.copies.progress[0].state,
            CopyOperationState::CancelRequested
        );
        for state in [
            CopyOperationState::Running,
            CopyOperationState::WaitingForDecision,
        ] {
            let mut progress = (*p.copies.progress[0]).clone();
            progress.state = state;
            p.apply(Event::Copy(CopyEvent::Progress(Arc::new(progress))));
            failure(&mut p, 8, true, true);
            let item = MutationItem {
                path: "destination/item".into(),
                kind: EntryKind::File,
                metadata: EntryMetadata::default(),
            };
            p.apply(Event::Copy(CopyEvent::Conflict(Arc::new(CopyConflict {
                id: OperationId(1),
                version: 9,
                source: item.clone(),
                target: item,
                destination: "destination/item".into(),
                can_replace: true,
                can_merge: false,
            }))));
            assert!(p.active_copy_decision().is_none());
            assert_eq!(
                p.copies.progress[0].state,
                CopyOperationState::CancelRequested
            );
        }
        let mut done = (*p.copies.progress[0]).clone();
        done.state = CopyOperationState::Cancelled;
        p.apply(Event::Copy(CopyEvent::Progress(Arc::new(done))));
        assert_eq!(p.copies.progress[0].state, CopyOperationState::Cancelled);
    }
}

#[test]
fn refused_cancel_keeps_decision_live_and_backend_cancel_requested_closes_it() {
    let mut p = presenter();
    failure(&mut p, 7, true, true);
    p.cancel_copy("copy-job:1", &|_| false);
    assert_eq!(p.active_copy_decision(), Some((OperationId(1), 7)));
    let mut cancelling = (*p.copies.progress[0]).clone();
    cancelling.state = CopyOperationState::CancelRequested;
    p.apply(Event::Copy(CopyEvent::Progress(Arc::new(cancelling))));
    failure(&mut p, 8, true, true);
    assert!(p.active_copy_decision().is_none());
}
