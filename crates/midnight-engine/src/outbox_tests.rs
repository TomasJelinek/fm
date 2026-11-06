use super::*;
use std::time::Duration;

#[test]
fn cancellation_releases_a_worker_even_while_the_ui_result_queue_is_full() {
    let (sender, receiver) = mpsc::channel(1);
    sender
        .try_send(Event::FontFamiliesLoaded(Ok(Vec::new())))
        .unwrap();
    let outbox = Outbox {
        sender,
        wakeup: Arc::new(|| {}),
    };
    let cancel = Cancellation::default();
    let worker_cancel = cancel.clone();
    let (entered, started) = std::sync::mpsc::channel();
    let (finished, result) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        entered.send(()).unwrap();
        let sent = outbox.blocking_send(Event::FontFamiliesLoaded(Ok(Vec::new())), &worker_cancel);
        finished.send(sent).unwrap();
    });
    started.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(matches!(
        result.recv_timeout(Duration::from_millis(30)),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));
    cancel.cancel();
    let cancelled_result = result.recv_timeout(Duration::from_secs(1));
    drop(receiver); // Always release a regressed blocking sender before joining.
    worker.join().unwrap();
    assert!(!cancelled_result.unwrap());
}
