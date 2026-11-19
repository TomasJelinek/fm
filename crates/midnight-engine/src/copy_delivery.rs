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
    /// Intermediate events only: `CopyOutbox::deliver` publishes a job's terminal
    /// progress once, so this refuses terminal progress from any other publisher.
    #[expect(
        clippy::result_large_err,
        reason = "Keep refused events intact for retry or disposal without an additional box."
    )]
    pub(super) fn try_send(&self, event: Event) -> Result<(), TrySendError<Event>> {
        if let Event::Copy(CopyEvent::Progress(progress)) = &event
            && progress.state.is_terminal()
        {
            return Err(TrySendError::Closed(event));
        }
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
    pub(super) pause: Arc<CopyPause>,
}
impl CopyOutbox {
    #[cfg(test)]
    pub(super) fn pending(outbox: Outbox) -> Self {
        Self::pending_with_shutdown(
            outbox,
            Cancellation::default(),
            Controls::default(),
            Cancellation::default().pausable_child(),
        )
    }
    pub(super) fn pending_with_shutdown(
        outbox: Outbox,
        shutdown: Cancellation,
        controls: Controls,
        cancel: Cancellation,
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
            pause: Arc::new(CopyPause::new(cancel)),
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
        progress.cancel_confirmation_ready = None;
        progress.wait_reason = None;
        progress.verification_pass_id = 0;
        progress.verification_throughput = Arc::default();
        progress.closure = closure(&progress);
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

/// The terminal result arrives after checked worker retirement. Keep its existing
/// errors and recovery paths on that exact snapshot and add only the final counts.
fn closure(progress: &CopyProgress) -> Option<CopyClosureReceipt> {
    if !progress.state.is_terminal()
        || (progress.state == CopyOperationState::Completed
            && progress.failed == 0
            && progress.skipped == 0
            && progress.errors.is_empty()
            && progress.source_delete_pending == 0
            && progress.source_removal_durability_unconfirmed == 0
            && progress.source_residual_count == 0
            && progress.recovery_count == 0)
    {
        return None;
    }
    let discovery = &progress.discovery;
    let recursive =
        discovery.scope == DiscoveryScope::Recursive && discovery.state == DiscoveryState::Complete;
    Some(CopyClosureReceipt {
        unfinished_roots: progress
            .requested_roots
            .saturating_sub(progress.completed_roots),
        uncommitted_files: recursive
            .then(|| discovery.files.checked_sub(progress.files_completed))
            .flatten(),
        uncommitted_bytes: discovery
            .bytes
            .filter(|_| recursive)
            .and_then(|bytes| bytes.checked_sub(progress.completed_bytes)),
        omitted_error_details: progress.failed.saturating_sub(progress.errors.len() as u64),
    })
}

#[cfg(test)]
#[path = "copy_delivery_tests.rs"]
mod tests;
