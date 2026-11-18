//! External outcome fences through actual Session and Mailbox boundaries.
use super::*;
use std::sync::mpsc::{Receiver, TryRecvError};

fn session(handle: &tokio::runtime::Handle, capacity: usize) -> (Session, Receiver<Message>, u64) {
    let generation = GENERATIONS.next().unwrap();
    let (sender, receiver) = mpsc::sync_channel(capacity);
    let (_owner, spawner) = crate::resident_test_support::workers();
    let worker = spawner.spawn(handle, Box::new(|| {})).ok().unwrap();
    (
        Session {
            inbox: Arc::new(Inbox::new(sender)),
            cancelled: Arc::new(AtomicBool::new(false)),
            picker_cancel: Cancellation::default(),
            mailbox: Arc::new(Mailbox::new(
                generation,
                Palette::from(&crate::appearance_tests::changed()),
            )),
            handle: handle.clone(),
            worker,
        },
        receiver,
        generation,
    )
}

fn paths() -> [PathBuf; 2] {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    [base.join("before 文档.cpp"), base.join("after e\u{301}.c")]
}

fn geometry() -> ViewerGeometry {
    ViewerGeometry {
        width: 800,
        height: 300,
        scale: 1.0,
    }
}

fn unknown() -> Output {
    Output::ChooseFiles(FileChoice {
        paths: paths(),
        message: "Native launch returned an uncertain outcome.".into(),
        is_error: true,
        external_outcome: Some(ExternalCompareChoiceOutcome::Unknown),
    })
}

fn frame(generation: u64) -> Output {
    Output::Frame {
        names: ["left".into(), "right".into()],
        appearance_revision: 0,
        frame: Frame {
            fonts: None,
            text_family: None,
            geometry_revision: 0,
            comparison_generation: generation,
            identity: IdentityVerdict::Pending,
            mode: None,
            width: 2,
            height: 1,
            pixels: vec![0; 8],
            layers: None,
            accessible_text: String::new(),
            status: "obsolete built-in frame".into(),
            image: false,
            position: 0.0,
        },
    }
}

#[test]
fn accepted_latest_palette_hides_an_unread_frame_until_worker_replaces_it() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (session, _received, generation) = session(runtime.handle(), 2);
    let mut new = crate::appearance_tests::changed();
    new.background = [3, 5, 7, 255];
    let first = frame(generation);
    session.mailbox.put(generation, first);
    assert_eq!(session.restore_palette(&new).unwrap(), 1);
    assert!(session.mailbox.take().is_none());
    assert!(matches!(
        session.mailbox.state.lock().unwrap().output,
        Some(Output::Frame {
            appearance_revision: 0,
            ..
        })
    ));
    assert!(!session.mailbox.put(generation, frame(generation)));
    let mut replacement = frame(generation);
    if let Output::Frame {
        appearance_revision,
        ..
    } = &mut replacement
    {
        *appearance_revision = 1;
    }
    assert!(session.mailbox.put(generation, replacement));
    assert!(matches!(
        session.mailbox.take(),
        Some(Output::Frame {
            appearance_revision: 1,
            ..
        })
    ));
}

#[test]
fn palette_revision_exhaustion_and_disconnection_preserve_the_last_request() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (session, _received, _generation) = session(runtime.handle(), 2);
    let original = crate::appearance_tests::changed();
    let mut changed = original.clone();
    changed.selection = [9, 8, 7, 255];
    session.mailbox.state.lock().unwrap().appearance.revision = u64::MAX - 1;

    assert_eq!(session.restore_palette(&changed).unwrap(), u64::MAX);
    assert!(session.restore_palette(&original).is_err());
    assert_eq!(session.restore_palette(&changed).unwrap(), u64::MAX);
    session.mailbox.finish_on_worker(None);
    assert!(session.restore_palette(&changed).is_err());
    let accepted = session.mailbox.appearance();
    assert_eq!(accepted.revision, u64::MAX);
    assert!(accepted.palette == Palette::from(&changed));
}

#[test]
fn closed_palette_submission_does_not_advance_the_revision() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (session, _received, _generation) = session(runtime.handle(), 2);
    let mut changed = crate::appearance_tests::changed();
    changed.accent = [1, 2, 3, 255];
    session.close();
    assert!(session.restore_palette(&changed).is_err());
    assert_eq!(session.mailbox.appearance().revision, 0);
}

#[test]
fn external_pending_rejects_compare_and_browse_before_queueing_or_advancing_generation() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (session, received, generation) = session(runtime.handle(), 32);
    assert!(session.mailbox.begin_external(generation));
    assert!(session.send(Input::Compare(paths())).is_err());
    assert!(
        session
            .send(Input::Browse {
                side: 0,
                paths: paths(),
                owner: PathPickerOwner::None,
            })
            .is_err()
    );
    assert!(matches!(received.try_recv(), Err(TryRecvError::Empty)));

    assert!(session.send(Input::Resize(geometry(), 7)).is_ok());
    let Message::Input(command) = received.try_recv().unwrap() else {
        panic!("expected a queued input");
    };
    assert_eq!(command.generation, generation);
    assert!(matches!(command.input, Input::Resize(_, 7)));
    session.mailbox.put(
        generation,
        Output::ExternalStarted {
            pid: 101,
            tracking_warning: None,
        },
    );
    assert!(matches!(
        session.mailbox.take(),
        Some(Output::ExternalStarted { pid: 101, .. })
    ));
    assert!(matches!(received.try_recv(), Err(TryRecvError::Empty)));
}

#[test]
fn started_survives_status_frame_and_finish_and_is_delivered_only_once() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (session, received, generation) = session(runtime.handle(), 32);
    assert!(session.mailbox.begin_external(generation));
    session.mailbox.put(
        generation,
        Output::ExternalStarted {
            pid: 202,
            tracking_warning: Some("independent tracking".into()),
        },
    );
    session
        .mailbox
        .put(generation, Output::Status("late status".into()));
    session.mailbox.put(generation, frame(generation));
    session
        .mailbox
        .finish_on_worker(Some("worker ended".into()));
    assert!(matches!(session.mailbox.take(),
        Some(Output::ExternalStarted { pid: 202, tracking_warning: Some(message) })
            if message == "independent tracking"));

    session
        .mailbox
        .put(generation, Output::Status("after take".into()));
    session.mailbox.put(generation, unknown());
    session.mailbox.put(generation, frame(generation));
    session
        .mailbox
        .finish_on_worker(Some("after take error".into()));
    assert!(session.mailbox.take().is_none());
    assert!(session.send(Input::Compare(paths())).is_err());
    assert!(
        session
            .send(Input::Browse {
                side: 1,
                paths: paths(),
                owner: PathPickerOwner::None,
            })
            .is_err()
    );
    assert!(matches!(received.try_recv(), Err(TryRecvError::Empty)));
}

#[test]
fn unknown_survives_take_and_failed_retry_until_an_explicit_retry_is_accepted() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (session, received, generation) = session(runtime.handle(), 1);
    assert!(session.mailbox.begin_external(generation));
    session.mailbox.put(generation, unknown());
    assert!(
        matches!(session.mailbox.take(), Some(Output::ChooseFiles(choice))
        if choice.external_outcome == Some(ExternalCompareChoiceOutcome::Unknown)
            && choice.paths == paths())
    );

    session
        .mailbox
        .put(generation, Output::Status("after take".into()));
    session.mailbox.put(generation, unknown());
    session.mailbox.put(generation, frame(generation));
    assert!(session.mailbox.take().is_none());
    assert!(session.send(Input::Resize(geometry(), 8)).is_ok());
    assert!(session.send(Input::Compare(paths())).is_err());
    assert!(
        session
            .send(Input::Browse {
                side: 0,
                paths: paths(),
                owner: PathPickerOwner::None,
            })
            .is_err()
    );
    session
        .mailbox
        .put(generation, Output::Status("still unknown".into()));
    assert!(session.mailbox.take().is_none());

    let Message::Input(resize) = received.try_recv().unwrap() else {
        panic!("expected a queued input");
    };
    assert_eq!(resize.generation, generation);
    assert!(matches!(resize.input, Input::Resize(_, 8)));
    assert!(session.send(Input::Compare(paths())).is_ok());
    let Message::Input(retry) = received.try_recv().unwrap() else {
        panic!("expected a queued input");
    };
    assert!(retry.generation > generation);
    assert!(matches!(retry.input, Input::Compare(value) if value == paths()));
    assert!(matches!(session.mailbox.take(), Some(Output::Status(_))));
    assert!(session.mailbox.begin_external(retry.generation));
}

#[test]
fn explicit_browse_can_replace_a_consumed_unknown_but_never_implicitly_relaunches() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (session, received, generation) = session(runtime.handle(), 2);
    assert!(session.mailbox.begin_external(generation));
    session.mailbox.put(generation, unknown());
    let _ = session.mailbox.take();
    assert!(
        session
            .send(Input::Browse {
                side: 1,
                paths: paths(),
                owner: PathPickerOwner::None,
            })
            .is_ok()
    );
    let Message::Input(browse) = received.try_recv().unwrap() else {
        panic!("expected a queued input");
    };
    assert!(browse.generation > generation);
    assert!(
        matches!(browse.input, Input::Browse { side: 1, paths: value, .. } if value == paths())
    );
    assert!(matches!(received.try_recv(), Err(TryRecvError::Empty)));
    assert!(session.mailbox.begin_external(browse.generation));
}

#[test]
fn a_replacement_accepted_before_external_fence_prevents_stale_submission() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (session, received, generation) = session(runtime.handle(), 2);
    assert!(session.send(Input::Compare(paths())).is_ok());
    let Message::Input(retry) = received.try_recv().unwrap() else {
        panic!("expected a queued input");
    };
    assert!(retry.generation > generation);
    assert!(!session.mailbox.begin_external(generation));
    assert!(session.mailbox.begin_external(retry.generation));
    assert!(!session.mailbox.begin_external(retry.generation));
}

#[test]
fn close_cancels_pending_external_and_suppresses_every_late_outcome() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (session, _received, generation) = session(runtime.handle(), 2);
    let cancel = Cancellation::observing(session.cancelled.clone());
    assert!(session.mailbox.begin_external(generation));
    assert!(session.send(Input::Close).is_ok());
    assert!(cancel.is_cancelled());
    session.mailbox.put(
        generation,
        Output::ExternalStarted {
            pid: 303,
            tracking_warning: None,
        },
    );
    session.mailbox.put(generation, unknown());
    session
        .mailbox
        .finish_on_worker(Some("late close failure".into()));
    assert!(session.mailbox.take().is_none());
    assert!(session.send(Input::Compare(paths())).is_err());
}
