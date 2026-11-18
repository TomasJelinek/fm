//! Reserve the existing pending retirement batch before taking UI-owned captures.
//!
//! One scope covers one finite synchronous action or backend event, never an await,
//! timer or sequence of user actions. Keep a reclaimer alive through the post-window
//! final drain; dropping the final handle with Ready data would drop that data here.
use midnight_core::Command;
use midnight_engine::{CommandSendError, CommandSender, reclaim};
use midnight_ui::{BeginOwned, DeferredDrop, OwnedCommandScope};
use std::{cell::RefCell, rc::Rc};
use tokio::sync::mpsc;

enum PendingBatch {
    Empty,
    Claimed(reclaim::Retired),
    Ready(reclaim::Retired),
}

#[derive(Clone)]
pub(crate) struct CommandReclaim {
    queue: mpsc::Sender<reclaim::Retired>,
    pending: Rc<RefCell<PendingBatch>>,
    /// Told when an owned send is refused or no scope can be claimed; never borrows.
    refused: Rc<dyn Fn()>,
}

impl CommandReclaim {
    /// Reuse the queue from `reclaim::start`; this creates no task or second queue.
    pub(crate) fn new(queue: mpsc::Sender<reclaim::Retired>) -> Self {
        Self {
            queue,
            pending: Rc::new(RefCell::new(PendingBatch::Empty)),
            refused: Rc::new(|| {}),
        }
    }

    /// Report refused owned sends and unavailable scopes: the work behind them waits
    /// for the retry pass.
    pub(crate) fn observe_refusals(mut self, observer: Rc<dyn Fn()>) -> Self {
        self.refused = observer;
        self
    }

    /// Claim before constructing a request, taking a capture or dequeuing an event.
    /// None means the ownership-bearing action must not start.
    pub(crate) fn begin(&self, sender: &CommandSender) -> Option<Box<dyn OwnedCommandScope>> {
        self.claim(sender.clone())
    }

    pub(crate) fn factory(&self, sender: CommandSender) -> BeginOwned {
        let reclaim = self.clone();
        Rc::new(move || reclaim.begin(&sender))
    }

    /// Claimed is busy; Full/Closed leaves the same Ready batch in the pending slot.
    pub(crate) fn flush(&self) -> bool {
        let Ok(mut pending) = self.pending.try_borrow_mut() else {
            return false;
        };
        if matches!(&*pending, PendingBatch::Claimed(_)) {
            return false;
        }
        let PendingBatch::Ready(retired) = std::mem::replace(&mut *pending, PendingBatch::Empty)
        else {
            return true;
        };
        match self.queue.try_send(retired) {
            Ok(()) => true,
            Err(error) => {
                *pending = PendingBatch::Ready(error.into_inner());
                false
            }
        }
    }

    /// Only for the post-window final drain, whose caller retires this batch off UI.
    /// A current callback's claim cannot be extracted or displaced by final draining.
    pub(crate) fn take_ready(&self) -> Option<reclaim::Retired> {
        let mut pending = self.pending.try_borrow_mut().ok()?;
        if !matches!(&*pending, PendingBatch::Ready(_)) {
            return None;
        }
        match std::mem::replace(&mut *pending, PendingBatch::Empty) {
            PendingBatch::Ready(retired) => Some(retired),
            _ => unreachable!("only Ready was selected"),
        }
    }

    fn claim<T: Transport + 'static>(&self, sender: T) -> Option<Box<dyn OwnedCommandScope>> {
        let scope = self.try_claim(sender);
        if scope.is_none() {
            (self.refused)();
        }
        scope
    }

    fn try_claim<T: Transport + 'static>(&self, sender: T) -> Option<Box<dyn OwnedCommandScope>> {
        if !self.flush() {
            return None;
        }
        let mut pending = self.pending.try_borrow_mut().ok()?;
        if !matches!(&*pending, PendingBatch::Empty) {
            return None;
        }
        *pending = PendingBatch::Claimed(reclaim::Retired::new());
        Some(Box::new(Scope {
            reclaim: self.clone(),
            sender,
        }))
    }
}

// The production implementation always calls the selected typed Engine sender.
// A private fake implementation in the test module models ownership refusals only.
trait Transport {
    #[expect(
        clippy::result_large_err,
        reason = "Return the intact refused Command to reclamation without boxing or cloning."
    )]
    fn try_send(&self, command: Command) -> Result<(), CommandSendError>;
}

impl Transport for CommandSender {
    fn try_send(&self, command: Command) -> Result<(), CommandSendError> {
        CommandSender::try_send(self, command)
    }
}

struct Scope<T> {
    reclaim: CommandReclaim,
    sender: T,
}

impl<T: Transport> OwnedCommandScope for Scope<T> {
    fn send(&self, command: Command) -> bool {
        match self.sender.try_send(command) {
            Ok(()) => true,
            Err(error) => {
                self.retire(Box::new(error.into_inner()));
                (self.reclaim.refused)();
                false
            }
        }
    }

    fn retire(&self, value: DeferredDrop) {
        let mut pending = self.reclaim.pending.borrow_mut();
        let PendingBatch::Claimed(retired) = &mut *pending else {
            unreachable!("only the live scope owns and may fill Claimed");
        };
        retired.push(value);
    }
}

impl<T> Drop for Scope<T> {
    fn drop(&mut self) {
        {
            let mut pending = self.reclaim.pending.borrow_mut();
            let PendingBatch::Claimed(retired) = &mut *pending else {
                unreachable!("a live scope's pending claim cannot be stolen");
            };
            if retired.is_empty() {
                *pending = PendingBatch::Empty;
                return;
            }
            let retired = std::mem::take(retired);
            *pending = PendingBatch::Ready(retired);
        }
        self.reclaim.flush();
    }
}

#[cfg(test)]
#[path = "command_reclaim_tests.rs"]
mod tests;
