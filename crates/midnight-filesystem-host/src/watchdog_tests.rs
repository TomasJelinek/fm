use super::*;

#[test]
fn sustained_progress_can_outlive_old_total_deadline_but_idle_and_cancel_stop() {
    let state = Supervision::new(None);
    let start = Instant::now();
    let cancel = Cancellation::default();
    for seconds in (0..=190).step_by(19) {
        state.progress_at(start + Duration::from_secs(seconds), Some(&cancel));
        assert!(!state.expired_at(start + Duration::from_secs(seconds + 19)));
    }
    assert!(state.expired_at(start + Duration::from_secs(211)));
    state.progress_at(start + Duration::from_secs(220), Some(&cancel));
    cancel.cancel();
    assert!(state.expired_at(start + Duration::from_secs(220)));
    // A token-free exchange ignores request cancellation and ends only by idle expiry.
    state.progress_at(start + Duration::from_secs(220), None);
    assert!(!state.expired_at(start + Duration::from_secs(239)));
    assert!(state.expired_at(start + Duration::from_secs(240)));
}

#[test]
fn idle_supervisor_blocks_until_teardown_or_an_armed_exchange_expires() {
    let state = Arc::new(Supervision::new(None));
    let (done, finished) = std::sync::mpsc::sync_channel(1);
    let waiting = state.clone();
    let supervisor = std::thread::spawn(move || {
        waiting.wait_for_stop_or_expiry();
        done.send(()).unwrap();
    });
    // An idle watch has no deadline; arming one must wake the waiting supervisor.
    let cancel = Cancellation::default();
    state.begin(Instant::now(), Some(&cancel));
    cancel.cancel();
    finished.recv_timeout(Duration::from_secs(10)).unwrap();
    supervisor.join().unwrap();
    // Teardown wakes a supervisor that is idle with no armed exchange.
    let state = Arc::new(Supervision::new(None));
    let waiting = state.clone();
    let (done, finished) = std::sync::mpsc::sync_channel(1);
    let supervisor = std::thread::spawn(move || {
        waiting.wait_for_stop_or_expiry();
        done.send(()).unwrap();
    });
    state.request_stop(Stop::Closed);
    finished.recv_timeout(Duration::from_secs(10)).unwrap();
    supervisor.join().unwrap();
    assert!(state.stopped());
}

#[test]
fn later_requests_report_why_the_worker_was_stopped() {
    // Idle-progress expiry is a failure that names the deadline, not a cancellation.
    let expired = Supervision::new(Some(Watch {
        deadline: Instant::now(),
        cancel: None,
    }));
    expired.wait_for_stop_or_expiry();
    let error = expired.stopped_error();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert!(
        error.to_string().contains("no progress for 20 seconds"),
        "{error}"
    );
    // Only the hard cancellation of startup, by the caller that owns the mount, is one.
    let cancel = Cancellation::default();
    cancel.cancel();
    let cancelled = Supervision::new(Some(Watch {
        deadline: Instant::now() + DEADLINE,
        cancel: Some(cancel),
    }));
    cancelled.wait_for_stop_or_expiry();
    assert_eq!(cancelled.stopped_error().kind(), io::ErrorKind::Interrupted);
    // A failed exchange is named by its failure; the first cause is kept, so a later
    // teardown or expiry does not replace it.
    let failed = Supervision::new(Some(Watch {
        deadline: Instant::now(),
        cancel: None,
    }));
    failed.request_stop(Stop::failed("invalid data"));
    failed.request_stop(Stop::Closed);
    failed.wait_for_stop_or_expiry();
    let error = failed.stopped_error();
    assert_eq!(error.kind(), io::ErrorKind::Other);
    assert!(error.to_string().contains("(invalid data)"), "{error}");
    let closed = Supervision::new(None);
    closed.request_stop(Stop::Closed);
    assert_eq!(closed.stopped_error().kind(), io::ErrorKind::Other);
    assert_ne!(closed.stopped_error().kind(), io::ErrorKind::Interrupted);
    // A retained failure message is bounded at a character boundary.
    let long = "é".repeat(400);
    let Stop::Failed(kept) = Stop::failed(&long) else {
        unreachable!("a failure keeps its message")
    };
    assert!(
        kept.len() <= 512 && long.starts_with(&kept),
        "{}",
        kept.len()
    );
}
