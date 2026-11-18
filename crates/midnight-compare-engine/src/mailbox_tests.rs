use super::*;

#[test]
fn generation_exhaustion_never_wraps_to_an_old_attempt() {
    let clock = GenerationClock(AtomicU64::new(u64::MAX - 1));
    assert_eq!(clock.next().unwrap(), u64::MAX);
    assert!(clock.next().is_err());
    assert!(clock.next().is_err());
    assert_eq!(clock.0.load(Ordering::Acquire), u64::MAX);
}

#[test]
fn late_worker_status_cannot_overwrite_another_attempt_or_closed_session() {
    let mailbox = Mailbox::new(7, Palette::default());
    mailbox.put(7, Output::Status("current".into()));
    mailbox.put(6, Output::Status("stale".into()));
    assert!(matches!(mailbox.take(), Some(Output::Status(status)) if status == "current"));
    mailbox.close();
    mailbox.put(7, Output::Status("late".into()));
    assert!(mailbox.take().is_none());
}

#[test]
fn terminal_worker_error_replaces_pending_status_and_respects_close() {
    let mailbox = Mailbox::new(12, Palette::default());
    mailbox.put(12, Output::Status("Preparing file identities…".into()));
    mailbox.finish_on_worker(Some("Worker failed".into()));
    assert!(matches!(mailbox.take(), Some(Output::Status(status)) if status == "Worker failed"));
    mailbox.close();
    mailbox.finish_on_worker(Some("Late failure".into()));
    assert!(mailbox.take().is_none());
}
