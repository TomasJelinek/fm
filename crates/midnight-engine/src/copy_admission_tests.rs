use super::*;

fn request(admission: &Admission, id: u64) -> (Request, oneshot::Sender<Request>) {
    let reservation = admission.reserve().unwrap();
    let command = CopyInput::from(CopyRequest {
        start_policy: TransferStartPolicy::Immediate,
        verification: TransferVerification::default(),
        id: OperationId(id),
        clipboard: FileClipboard {
            paths: vec![Path::new("source").to_owned()],
            intent: FileClipboardIntent::Copy,
        },
        destination: Path::new("target").to_owned(),
    });
    let initial = initial(&command);
    let status = Arc::new(Mutex::new(initial.clone()));
    let (_, decisions) = mpsc::channel(1);
    let (sender, _events) = mpsc::channel(1);
    let events = CopyOutbox::pending(Outbox {
        sender,
        wakeup: Arc::new(|| {}),
    });
    reservation
        .attach(Input {
            command,
            ordered_id: true,
            cancel: Cancellation::default(),
            status,
            decisions,
            waiting: Arc::new(AtomicU64::new(0)),
            initial,
            events,
        })
        .unwrap_or_else(|_| panic!("pure input attachment quarantined"))
}

#[tokio::test]
async fn later_ready_payload_bypasses_an_unfilled_reservation() {
    let admission = Admission::default();
    let earlier = admission.reserve().unwrap();
    let (later, sender) = request(&admission, 9);
    sender
        .send(later)
        .unwrap_or_else(|_| panic!("receiver closed"));
    let mut received = tokio::time::timeout(
        Duration::from_secs(2),
        admission.next(&Cancellation::default()),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(received.command.id, OperationId(9));
    let predecessors = received.resources_mut().take_predecessors();
    assert!(
        !predecessors.is_finished(),
        "bypass must not finish the earlier reservation"
    );
    assert_eq!(admission.0.state.lock().unwrap().slots.len(), 1);
    drop(earlier);
    let _ = received.retire().await;
}

#[tokio::test]
async fn closed_hole_does_not_finish_unconfirmed_owner_or_block_a_ready_payload() {
    let admission = Admission::default();
    let abandoned = admission.reserve_slot().unwrap();
    drop(abandoned);
    let (later, sender) = request(&admission, 1);
    sender
        .send(later)
        .unwrap_or_else(|_| panic!("receiver closed"));
    let mut received = admission.next(&Cancellation::default()).await.unwrap();
    assert!(!received.resources_mut().take_predecessors().is_finished());
    assert!(admission.0.state.lock().unwrap().slots.is_empty());
    let _ = received.retire().await;
}

#[tokio::test]
async fn cancelled_consumer_does_not_take_a_filled_payload() {
    let admission = Admission::default();
    let (payload, sender) = request(&admission, 1);
    sender
        .send(payload)
        .unwrap_or_else(|_| panic!("receiver closed"));
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(admission.next(&cancel).await.is_none());
    let received = admission.next(&Cancellation::default()).await.unwrap();
    assert_eq!(received.command.id, OperationId(1));
    let _ = received.retire().await;
}

#[tokio::test]
async fn consumer_close_retires_payloads_without_finishing_unfilled_owner() {
    let admission = Admission::default();
    let earlier = admission.reserve().unwrap();
    let (mut later, sender) = request(&admission, 2);
    let predecessors = later.resources_mut().take_predecessors();
    sender
        .send(later)
        .unwrap_or_else(|_| panic!("receiver closed"));
    drop(admission.consumer());
    assert!(!predecessors.is_finished());
    assert!(admission.reserve().is_err());
    drop(earlier);
}

struct HeldProvider {
    entered: std::sync::mpsc::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
}

// This fixture owns only two host channels and performs its complete bounded
// destructor synchronously; it hides no deferred worker or native owner.
impl crate::copy_nested_retirement::CheckedRetirement for HeldProvider {
    fn retirement_is_checked(&self) -> bool {
        true
    }
}

impl Drop for HeldProvider {
    fn drop(&mut self) {
        self.entered.send(()).unwrap();
        self.release.recv_timeout(Duration::from_secs(4)).unwrap();
    }
}

#[tokio::test]
async fn abandoned_native_reservation_finishes_only_after_retained_provider_destruction() {
    let admission = Admission::default();
    let mut earlier = admission.reserve().unwrap();
    let (entered, entered_rx) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel();
    earlier
        .owned
        .resources_mut()
        .retain_checked_provider(Box::new(HeldProvider {
            entered,
            release: released,
        }));
    let (mut later, sender) = request(&admission, 2);
    let predecessors = later.resources_mut().take_predecessors();
    let success = admission.fence().snapshot();
    drop(earlier);
    let entered = entered_rx.recv_timeout(Duration::from_secs(4));
    let pending_while_drop_held = !predecessors.is_finished();
    let capacity_while_drop_held = admission.capacity().available_permits();
    let released = release.send(());
    entered.unwrap();
    released.unwrap();
    let cancel = Cancellation::default();
    assert!(
        tokio::time::timeout(Duration::from_secs(2), predecessors.wait(&cancel))
            .await
            .unwrap()
    );
    assert!(pending_while_drop_held);
    assert_eq!(capacity_while_drop_held, MAX_JOBS - 2);
    assert!(success.wait(&cancel, Path::new("target")).await.is_err());
    drop(sender);
    let _ = later.retire().await;
}
