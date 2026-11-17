//! Ordered, provider-free delivery never prevents admitted native work from starting.
use super::*;
use std::sync::atomic::AtomicBool;
use tokio::sync::{mpsc::error::TrySendError, oneshot};

struct Initial {
    delivered: AtomicBool,
    state: Mutex<DeliveryState>,
}

#[derive(Default)]
struct DeliveryState {
    closed: bool,
}

#[derive(Clone)]
pub(super) struct Sender {
    sender: mpsc::Sender<Event>,
    initial: Arc<Initial>,
}
impl Sender {
    #[expect(
        clippy::result_large_err,
        reason = "Keep refused events intact for retry or disposal without an additional box."
    )]
    pub(super) fn try_send(&self, event: Event) -> Result<(), TrySendError<Event>> {
        let state = self
            .initial
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.closed {
            return Err(TrySendError::Closed(event));
        }
        if !self.initial.delivered.load(Ordering::Acquire) {
            return Err(TrySendError::Full(event));
        }
        self.sender.try_send(event)
    }
    fn close_intermediates(&self) {
        self.initial
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .closed = true;
    }
    #[cfg(test)]
    fn intermediates_closed(&self) -> bool {
        self.initial
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .closed
    }
}

#[derive(Clone)]
pub(super) struct CopyOutbox {
    pub sender: Sender,
    pub wakeup: Arc<dyn Fn() + Send + Sync>,
    notices: mpsc::Sender<Event>,
    receiver: Arc<Mutex<Option<mpsc::Receiver<Event>>>>,
    shutdown: Cancellation,
    /// Announces job controls and shutdown to every wait of this job.
    pub controls: Controls,
}
impl CopyOutbox {
    #[cfg(test)]
    pub(super) fn pending(outbox: Outbox) -> Self {
        Self::pending_with_shutdown(outbox, Cancellation::default(), Controls::default())
    }
    pub(super) fn pending_with_shutdown(
        outbox: Outbox,
        shutdown: Cancellation,
        controls: Controls,
    ) -> Self {
        // Public admission accepts at most 4096 roots. Root-completion notices
        // are provider-free; one decision notice plus one spare fits after all
        // roots even while initial UI delivery is blocked. Progress coalesces.
        let (notices, receiver) = mpsc::channel(4098);
        Self {
            sender: Sender {
                sender: outbox.sender,
                initial: Arc::new(Initial {
                    delivered: AtomicBool::new(false),
                    state: Mutex::new(DeliveryState::default()),
                }),
            },
            wakeup: outbox.wakeup,
            notices,
            receiver: Arc::new(Mutex::new(Some(receiver))),
            shutdown,
            controls,
        }
    }
    #[cfg(test)]
    pub(super) fn unblocked(outbox: Outbox) -> Self {
        let result = Self::pending(outbox);
        result
            .sender
            .initial
            .delivered
            .store(true, Ordering::Release);
        result
    }
    pub(super) async fn send(&self, event: Event) {
        if matches!(&event, Event::Copy(CopyEvent::Progress(_))) {
            if self.sender.try_send(event).is_ok() {
                (self.wakeup)();
            }
            return;
        }
        let Ok(permit) = self.notices.reserve().await else {
            return;
        };
        let state = self
            .sender
            .initial
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !state.closed {
            permit.send(event);
        }
    }
    pub(super) async fn deliver(
        self,
        initial: CopyProgress,
        terminal: oneshot::Receiver<CopyProgress>,
    ) {
        let notices = self
            .receiver
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        let Some(mut notices) = notices else {
            return;
        };
        let initial_sent = self
            .forward(Event::Copy(CopyEvent::Progress(Arc::new(initial))))
            .await;
        self.sender.initial.delivered.store(true, Ordering::Release);
        if !initial_sent {
            return;
        }
        tokio::pin!(terminal);
        let mut progress = loop {
            tokio::select! {
                notice = notices.recv() => {
                    if let Some(notice) = notice
                        && !self.forward(notice).await
                    {
                        return;
                    }
                },
                progress = &mut terminal => {
                    let Ok(progress) = progress else { return; };
                    break progress;
                },
                _ = self.wait_shutdown() => return,
            }
        };
        // Successful intermediate acceptance and sealing share one short mutex.
        // Accepted progress is in the global FIFO before terminal send can
        // reserve output capacity. Later stale snapshots are explicitly refused.
        self.sender.close_intermediates();
        // Native work enqueues every root notice before sending its terminal.
        // Drain those immutable notices before terminal delivery.
        while let Ok(notice) = notices.try_recv() {
            if !self.forward(notice).await {
                return;
            }
        }
        progress.pause_requested = false;
        progress.paused = false;
        progress.wait_reason = None;
        progress.verification_pass_id = 0;
        progress.verification_throughput = Arc::default();
        self.forward(Event::Copy(CopyEvent::Progress(Arc::new(progress))))
            .await;
    }
    async fn forward(&self, event: Event) -> bool {
        tokio::select! {
            result = self.sender.sender.send(event) => {
                if result.is_ok() {
                    (self.wakeup)();
                }
                result.is_ok()
            },
            _ = self.wait_shutdown() => false,
        }
    }
    /// Shutdown is announced through `controls`; delivery never polls for it.
    async fn wait_shutdown(&self) {
        self.controls.until(|| self.shutdown.is_cancelled()).await;
    }
}

#[cfg(test)]
#[path = "copy_delivery_tests.rs"]
mod tests;
