use super::*;
use midnight_core::{
    Cancellation, CapturedTransferRequest, CapturedTransferSources, FsError, OperationId,
    PreparedCapturedTransfer, TransferIntent, TransferService,
};
use std::{
    cell::Cell,
    sync::{Arc, mpsc as sync_mpsc},
    thread::ThreadId,
    time::Duration,
};
use tokio::sync::oneshot;

// Only the adapter's ownership behavior is modeled here, never Engine admission.
#[derive(Clone)]
struct FakeTransport(Rc<dyn Fn(Command) -> Result<(), CommandSendError>>);

impl Transport for FakeTransport {
    fn try_send(&self, command: Command) -> Result<(), CommandSendError> {
        (self.0)(command)
    }
}

#[expect(
    clippy::result_large_err,
    reason = "Model intact refused commands at the production reclamation boundary."
)]
fn full() -> FakeTransport {
    FakeTransport(Rc::new(|command| Err(CommandSendError::Full(command))))
}

#[derive(Debug)]
struct DropProbe {
    id: u64,
    dropped: Option<oneshot::Sender<(u64, ThreadId)>>,
}

impl Drop for DropProbe {
    fn drop(&mut self) {
        if let Some(dropped) = self.dropped.take() {
            let _ = dropped.send((self.id, std::thread::current().id()));
        }
    }
}

impl CapturedTransferSources for DropProbe {
    fn intent(&self) -> TransferIntent {
        TransferIntent::Copy
    }
    fn root_count(&self) -> usize {
        1
    }
    fn metadata_available(&self) -> bool {
        false
    }
    fn prepare(
        &self,
        _: &dyn TransferService,
        _: &Cancellation,
    ) -> Result<PreparedCapturedTransfer, FsError> {
        panic!("the reclaim adapter must not prepare captured sources")
    }
}

fn probe(id: u64) -> (DropProbe, oneshot::Receiver<(u64, ThreadId)>) {
    let (dropped, notice) = oneshot::channel();
    (
        DropProbe {
            id,
            dropped: Some(dropped),
        },
        notice,
    )
}

fn captured(id: u64) -> (Command, oneshot::Receiver<(u64, ThreadId)>) {
    let (source, notice) = probe(id);
    (
        Command::EnqueueCapturedTransfer(CapturedTransferRequest {
            id: OperationId(id),
            sources: Arc::new(source),
            destination: "target".into(),
            start_policy: Default::default(),
            verification: Default::default(),
            metadata: Default::default(),
        }),
        notice,
    )
}

async fn notice(receiver: oneshot::Receiver<(u64, ThreadId)>) -> (u64, ThreadId) {
    tokio::time::timeout(Duration::from_secs(5), receiver)
        .await
        .unwrap()
        .unwrap()
}

struct BlockWorker {
    started: Option<oneshot::Sender<()>>,
    release: sync_mpsc::Receiver<()>,
}

impl Drop for BlockWorker {
    fn drop(&mut self) {
        if let Some(started) = self.started.take() {
            let _ = started.send(());
        }
        // A failed assertion closes the release sender; the worker cannot hang forever.
        let _ = self.release.recv_timeout(Duration::from_secs(5));
    }
}

#[tokio::test]
async fn four_queued_batches_and_one_ready_batch_block_further_owner_construction() {
    let caller = std::thread::current().id();
    let queue = reclaim::start(&tokio::runtime::Handle::current(), Arc::new(|| {}));
    let (started, blocked) = oneshot::channel();
    let (release, wait) = sync_mpsc::channel();
    assert!(
        queue
            .try_send(vec![Box::new(BlockWorker {
                started: Some(started),
                release: wait,
            })])
            .is_ok()
    );
    tokio::time::timeout(Duration::from_secs(5), blocked)
        .await
        .unwrap()
        .unwrap();

    let reclaimer = CommandReclaim::new(queue.clone());
    let constructed = Cell::new(0);
    let mut notices = Vec::new();
    for id in 0..5 {
        let scope = reclaimer
            .claim(full())
            .expect("one finite action may claim Empty");
        constructed.set(constructed.get() + 1);
        let (command, dropped) = captured(id);
        notices.push(dropped);
        assert!(!scope.send(command));
        drop(scope);
    }
    assert_eq!(queue.capacity(), 0, "the original four-batch queue is full");
    assert!(matches!(&*reclaimer.pending.borrow(), PendingBatch::Ready(batch) if batch.len() == 1));
    for _ in 0..32 {
        if let Some(scope) = reclaimer.claim(full()) {
            constructed.set(constructed.get() + 1);
            scope.retire(Box::new(probe(99).0));
        }
    }
    assert_eq!(
        constructed.get(),
        5,
        "busy callbacks must not construct more owners"
    );
    assert!(
        notices
            .iter_mut()
            .all(|receiver| receiver.try_recv().is_err())
    );

    release.send(()).unwrap();
    let first = notice(notices.remove(0)).await;
    assert_eq!(first.0, 0);
    assert_ne!(first.1, caller);
    // Drop sends its notice before the reclaimer resumes from spawn_blocking.
    // Observe actual channel credit before asserting a nonblocking flush.
    let permit = tokio::time::timeout(Duration::from_secs(5), queue.reserve())
        .await
        .unwrap()
        .unwrap();
    drop(permit);
    assert!(reclaimer.flush());
    let empty = reclaimer
        .claim(full())
        .expect("capacity release allows the next action");
    drop(empty);
    tokio::time::timeout(Duration::from_secs(5), reclaim::barrier(&queue))
        .await
        .unwrap()
        .unwrap();
    for (index, receiver) in notices.into_iter().enumerate() {
        let (id, thread) = notice(receiver).await;
        assert_eq!(id, index as u64 + 1);
        assert_ne!(thread, caller);
    }
}

#[tokio::test]
#[expect(
    clippy::result_large_err,
    reason = "Keep exact Full/Closed command ownership visible to the reclamation worker test."
)]
async fn full_and_closed_transport_returned_commands_keep_the_exact_captured_owner() {
    let caller = std::thread::current().id();
    for closed in [false, true] {
        let (queue, receiver) = mpsc::channel::<reclaim::Retired>(4);
        for _ in 0..4 {
            assert!(queue.try_send(vec![]).is_ok());
        }
        let reclaimer = CommandReclaim::new(queue);
        let transport = FakeTransport(Rc::new(move |command| {
            Err(if closed {
                CommandSendError::Closed(command)
            } else {
                CommandSendError::Full(command)
            })
        }));
        let scope = reclaimer.claim(transport).unwrap();
        let (command, mut dropped) = captured(7);
        assert!(!scope.send(command));
        assert!(
            reclaimer.take_ready().is_none(),
            "a live claim cannot be stolen"
        );
        drop(scope);
        assert!(dropped.try_recv().is_err());
        assert!(!reclaimer.flush());
        assert!(reclaimer.claim(full()).is_none());
        let retired = reclaimer.take_ready().expect("post-loop drain owns Ready");
        assert_eq!(retired.len(), 1);
        drop(receiver);
        tokio::task::spawn_blocking(move || drop(retired))
            .await
            .unwrap();
        let (id, thread) = notice(dropped).await;
        assert_eq!(id, 7);
        assert_ne!(thread, caller);
    }
}

#[test]
#[expect(
    clippy::result_large_err,
    reason = "Match the inline production command transport contract in this ownership test."
)]
fn an_empty_or_reentrant_claim_neither_releases_nor_steals_the_pending_slot() {
    let (queue, _receiver) = mpsc::channel::<reclaim::Retired>(4);
    let reclaimer = CommandReclaim::new(queue);
    let nested = reclaimer.clone();
    let calls = Rc::new(Cell::new(0));
    let count = calls.clone();
    let transport = FakeTransport(Rc::new(move |command| {
        count.set(count.get() + 1);
        assert!(!nested.flush());
        assert!(nested.take_ready().is_none());
        assert!(nested.claim(full()).is_none());
        assert!(matches!(command, Command::Shutdown));
        Ok(())
    }));
    let scope = reclaimer.claim(transport).unwrap();
    assert!(scope.send(Command::Shutdown));
    assert_eq!(calls.get(), 1);
    assert!(!reclaimer.flush());
    drop(scope);
    assert!(reclaimer.flush());
    assert!(reclaimer.claim(full()).is_some());
}

#[test]
fn refused_owned_sends_and_unavailable_scopes_report_work_for_the_retry_pass() {
    let (queue, _receiver) = mpsc::channel::<reclaim::Retired>(4);
    let refusals = Rc::new(Cell::new(0));
    let counted = refusals.clone();
    let reclaimer = CommandReclaim::new(queue)
        .observe_refusals(Rc::new(move || counted.set(counted.get() + 1)));
    let scope = reclaimer.claim(full()).unwrap();
    assert_eq!(refusals.get(), 0, "a claimed scope reports nothing");
    assert!(!scope.send(Command::Shutdown));
    assert_eq!(refusals.get(), 1, "a refused send is reported");
    assert!(reclaimer.claim(full()).is_none());
    assert_eq!(refusals.get(), 2, "an unavailable scope is reported");
    drop(scope);
    assert!(reclaimer.claim(full()).is_some());
    assert_eq!(refusals.get(), 2);
}

#[tokio::test]
async fn a_closed_reclaim_queue_retains_ready_until_the_post_loop_worker_drain() {
    let caller = std::thread::current().id();
    let (queue, receiver) = mpsc::channel::<reclaim::Retired>(4);
    drop(receiver);
    let reclaimer = CommandReclaim::new(queue);
    let scope = reclaimer.claim(full()).unwrap();
    let (value, mut dropped) = probe(8);
    scope.retire(Box::new(value));
    drop(scope);
    assert!(!reclaimer.flush());
    assert!(reclaimer.claim(full()).is_none());
    assert!(dropped.try_recv().is_err());
    let retired = reclaimer.take_ready().unwrap();
    tokio::task::spawn_blocking(move || drop(retired))
        .await
        .unwrap();
    assert_ne!(notice(dropped).await.1, caller);
}

#[tokio::test]
#[expect(
    clippy::result_large_err,
    reason = "Match inline command results while checking fast backend release and scope cleanup."
)]
async fn fast_success_still_retires_the_original_draft_on_the_existing_worker() {
    let caller = std::thread::current().id();
    let queue = reclaim::start(&tokio::runtime::Handle::current(), Arc::new(|| {}));
    let reclaimer = CommandReclaim::new(queue.clone());
    let transport = FakeTransport(Rc::new(|command| {
        // Simulate completed backend retirement before the UI callback exits.
        std::thread::spawn(move || drop(command)).join().unwrap();
        Ok(())
    }));
    let scope = reclaimer.claim(transport).unwrap();
    let (command, mut dropped) = captured(9);
    let original = match &command {
        Command::EnqueueCapturedTransfer(request) => request.sources.clone(),
        _ => unreachable!(),
    };
    assert!(scope.send(command));
    assert!(
        dropped.try_recv().is_err(),
        "the original draft still owns its source"
    );
    scope.retire(Box::new(original));
    drop(scope);
    tokio::time::timeout(Duration::from_secs(5), reclaim::barrier(&queue))
        .await
        .unwrap()
        .unwrap();
    let (id, thread) = notice(dropped).await;
    assert_eq!(id, 9);
    assert_ne!(thread, caller);
}

#[tokio::test]
async fn the_adapter_operates_under_an_existing_mutable_presenter_borrow() {
    let caller = std::thread::current().id();
    let queue = reclaim::start(&tokio::runtime::Handle::current(), Arc::new(|| {}));
    let reclaimer = CommandReclaim::new(queue.clone());
    let presenter = RefCell::new(midnight_ui::Presenter::new("left".into(), "right".into()));
    let scope = reclaimer.claim(full()).unwrap();
    let (command, dropped) = captured(10);
    {
        let mut state = presenter.borrow_mut();
        assert!(!scope.send(command));
        for value in state.take_retired() {
            scope.retire(value);
        }
    }
    drop(scope);
    tokio::time::timeout(Duration::from_secs(5), reclaim::barrier(&queue))
        .await
        .unwrap()
        .unwrap();
    assert_ne!(notice(dropped).await.1, caller);
}

#[tokio::test]
async fn callback_unwind_hands_its_claim_to_the_existing_reclaim_worker() {
    let caller = std::thread::current().id();
    let queue = reclaim::start(&tokio::runtime::Handle::current(), Arc::new(|| {}));
    let reclaimer = CommandReclaim::new(queue.clone());
    let (value, dropped) = probe(11);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let scope = reclaimer.claim(full()).unwrap();
        scope.retire(Box::new(value));
        panic!("finite action failed after retiring its displaced owner");
    }));
    assert!(result.is_err());
    tokio::time::timeout(Duration::from_secs(5), reclaim::barrier(&queue))
        .await
        .unwrap()
        .unwrap();
    assert_ne!(notice(dropped).await.1, caller);
}
