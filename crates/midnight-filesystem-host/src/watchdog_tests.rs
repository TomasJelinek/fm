use super::*;

#[test]
fn sustained_progress_can_outlive_old_total_deadline_but_idle_and_cancel_stop() {
    let state = Supervision {
        stop: AtomicBool::new(false),
        deadline: Mutex::new(None),
    };
    let start = Instant::now();
    let cancel = Cancellation::default();
    for seconds in (0..=190).step_by(19) {
        state.progress_at(start + Duration::from_secs(seconds), &cancel);
        assert!(!state.expired_at(start + Duration::from_secs(seconds + 19)));
    }
    assert!(state.expired_at(start + Duration::from_secs(211)));
    state.progress_at(start + Duration::from_secs(220), &cancel);
    cancel.cancel();
    assert!(state.expired_at(start + Duration::from_secs(220)));
}
