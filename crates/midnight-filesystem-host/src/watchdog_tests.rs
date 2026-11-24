use super::*;

#[test]
fn sustained_progress_can_outlive_old_total_deadline_but_idle_and_cancel_stop() {
    let state = Supervision::new(None);
    let start = Instant::now();
    let cancel = Cancellation::default();
    let allowance = Duration::from_secs(60);
    state.begin(start, Some(Hard::Startup(&cancel)), allowance);
    for seconds in (0..=190).step_by(19) {
        state.progress_at(start + Duration::from_secs(seconds));
        assert_eq!(
            state.expired_at(start + Duration::from_secs(seconds + 19)),
            None
        );
    }
    assert_eq!(
        state.expired_at(start + Duration::from_secs(211)),
        Some(Expiry::Idle)
    );
    state.progress_at(start + Duration::from_secs(220));
    cancel.cancel();
    assert_eq!(
        state.expired_at(start + Duration::from_secs(220)),
        Some(Expiry::Cancelled)
    );
    // A token-free exchange ignores request cancellation and ends only by idle expiry.
    state.begin(start + Duration::from_secs(220), None, allowance);
    assert_eq!(state.expired_at(start + Duration::from_secs(239)), None);
    assert_eq!(
        state.expired_at(start + Duration::from_secs(240)),
        Some(Expiry::Idle)
    );
}

#[test]
fn source_reads_renew_the_idle_deadline_but_only_progress_renews_the_time_budget() {
    let state = Supervision::new(None);
    let start = Instant::now();
    let at = |seconds| start + Duration::from_secs(seconds);
    let allowance = Duration::from_secs(100);
    state.begin(start, None, allowance);
    // A worker that keeps requesting reads never idles, yet its budget runs out.
    for seconds in (0..100).step_by(10) {
        state.active_at(at(seconds));
        assert_eq!(state.expired_at(at(seconds + 9)), None);
    }
    assert_eq!(state.expired_at(at(100)), Some(Expiry::Time(allowance)));
    // A progress reply grants the whole budget again.
    state.begin(start, None, allowance);
    state.progress_at(at(90));
    state.active_at(at(180));
    assert_eq!(state.expired_at(at(189)), None);
    assert_eq!(state.expired_at(at(190)), Some(Expiry::Time(allowance)));
}

#[test]
fn a_cancelled_request_exchange_ends_after_its_grace_and_a_startup_at_once() {
    let state = Supervision::new(None);
    let grace = Budget::DEFAULT.cancel_grace;
    let start = Instant::now();
    let cancel = Cancellation::default();
    state.begin(
        start,
        Some(Hard::Request(&cancel)),
        Duration::from_secs(600),
    );
    assert_eq!(state.expired_at(start), None);
    cancel.cancel();
    // The grace counts from when the supervisor first sees the cancellation.
    let seen = start + Duration::from_secs(5);
    assert_eq!(state.expired_at(seen), None);
    assert_eq!(state.expired_at(seen + grace / 2), None);
    assert_eq!(
        state.expired_at(seen + grace),
        Some(Expiry::Abandoned(grace))
    );
    state.begin(
        start,
        Some(Hard::Startup(&cancel)),
        Duration::from_secs(600),
    );
    assert_eq!(state.expired_at(start), Some(Expiry::Cancelled));
}

#[test]
fn budgets_grow_with_the_archive_and_protocol_one_keeps_its_own() {
    let budget = Budget::DEFAULT;
    let gib = 1024 * 1024 * 1024;
    assert_eq!(budget.served(Protocol::V1, 64 << 20, 0), 64 << 20);
    assert_eq!(budget.served(Protocol::V4, 0, 0), gib);
    assert_eq!(budget.served(Protocol::V2, 10 * gib, 2 * gib), 49 * gib);
    assert_eq!(budget.served(Protocol::V4, u64::MAX, 1), u64::MAX);
    assert_eq!(budget.time(0), Duration::from_secs(600));
    // A 10 GiB archive: 10 minutes and 5,120 seconds.
    assert_eq!(budget.time(10 * gib), Duration::from_secs(600 + 5120));
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
    state.begin(
        Instant::now(),
        Some(Hard::Startup(&cancel)),
        Duration::from_secs(600),
    );
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
    // Idle-progress expiry is a failure that names the deadline, not a cancellation. It
    // is not retryable: only opening the archive again starts a worker.
    let expired = Supervision::new(Some(Watch::new(
        Instant::now(),
        Duration::ZERO,
        Duration::from_secs(600),
        None,
    )));
    expired.wait_for_stop_or_expiry();
    let error = expired.stopped_error();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(
        error.to_string().contains("no progress for 20 seconds"),
        "{error}"
    );
    assert!(
        error.to_string().contains("open the archive again"),
        "{error}"
    );
    // Only the hard cancellation of startup, by the caller that owns the mount, is one.
    let cancel = Cancellation::default();
    cancel.cancel();
    let cancelled = Supervision::new(Some(Watch::new(
        Instant::now(),
        DEADLINE,
        Duration::from_secs(600),
        Some(Hard::Startup(&cancel).watched(Duration::ZERO)),
    )));
    cancelled.wait_for_stop_or_expiry();
    assert_eq!(cancelled.stopped_error().kind(), io::ErrorKind::Interrupted);
    // A failed exchange is named by its failure; the first cause is kept, so a later
    // teardown or expiry does not replace it.
    let failed = Supervision::new(Some(Watch::new(
        Instant::now(),
        Duration::ZERO,
        Duration::from_secs(600),
        None,
    )));
    failed.request_stop(Stop::failed("invalid data"));
    failed.request_stop(Stop::Closed);
    failed.wait_for_stop_or_expiry();
    let error = failed.stopped_error();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("(invalid data)"), "{error}");
    assert!(
        error.to_string().contains("open the archive again"),
        "{error}"
    );
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
