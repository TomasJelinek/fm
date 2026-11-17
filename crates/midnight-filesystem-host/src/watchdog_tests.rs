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
    state.request_stop();
    finished.recv_timeout(Duration::from_secs(10)).unwrap();
    supervisor.join().unwrap();
    assert!(state.stopped());
}
