//! Cooperative job checkpoints. Cancellation checks remain nonblocking everywhere.
use super::Cancellation;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

#[derive(Debug, Default)]
pub(super) struct State {
    requested: AtomicBool,
    acknowledged: AtomicBool,
}
impl Cancellation {
    /// A child with its own job pause control. Descendants retain this control.
    /// Use one control per sequential operation, never a shared application lifetime.
    pub fn pausable_child(&self) -> Self {
        let mut child = self.child();
        let mut pauses = child.pauses.to_vec();
        pauses.push(Arc::default());
        child.pauses = pauses.into();
        child
    }
    /// Request a stop at the next safe worker checkpoint, or release that stop.
    /// This never waits for native I/O and never revokes cancellation.
    pub fn set_pause_requested(&self, requested: bool) {
        for pause in self.pauses.iter() {
            pause.requested.store(requested, Ordering::Release);
            if !requested {
                pause.acknowledged.store(false, Ordering::Release);
            }
        }
    }
    pub fn pause_requested(&self) -> bool {
        !self.is_cancelled()
            && self
                .pauses
                .iter()
                .any(|v| v.requested.load(Ordering::Acquire))
    }
    pub fn is_paused(&self) -> bool {
        self.pause_requested()
            && self
                .pauses
                .iter()
                .any(|v| v.acknowledged.load(Ordering::Acquire))
    }
    /// A nonblocking safe-point check for async workers. False means wait or cancel.
    /// Acknowledgement is emitted only when the worker reaches a safe boundary.
    pub fn pause_checkpoint(&self) -> bool {
        let cancelled = self.is_cancelled();
        let mut ready = !cancelled;
        for pause in self.pauses.iter() {
            let requested = !cancelled && pause.requested.load(Ordering::Acquire);
            pause.acknowledged.store(requested, Ordering::Release);
            ready &= !requested;
        }
        ready
    }
    /// Blocking-worker checkpoint. Resume and cancellation use a 10 ms polling cadence.
    /// Never call on an event loop, during cleanup, or while holding a progress lock.
    pub fn wait_if_paused(&self) -> bool {
        loop {
            if self.pause_checkpoint() {
                return true;
            }
            if self.is_cancelled() {
                return false;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::mpsc, time::Instant};
    fn await_paused(control: &Cancellation) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !control.is_paused() {
            assert!(Instant::now() < deadline);
            thread::yield_now();
        }
    }
    #[test]
    fn job_pause_is_acknowledged_only_at_checkpoint_and_resume_preserves_cancel() {
        let parent = Cancellation::default();
        let job = parent.pausable_child();
        let sibling = parent.pausable_child();
        job.set_pause_requested(true);
        assert!(job.pause_requested());
        assert!(!job.is_paused());
        assert!(!parent.pause_requested());
        assert!(!sibling.pause_requested());
        let worker = job.child();
        let (sent, received) = mpsc::channel();
        let thread = thread::spawn(move || sent.send(worker.wait_if_paused()).unwrap());
        await_paused(&job);
        assert!(received.try_recv().is_err());
        job.set_pause_requested(false);
        assert!(received.recv_timeout(Duration::from_secs(2)).unwrap());
        thread.join().unwrap();
        job.cancel();
        job.set_pause_requested(false);
        assert!(!job.wait_if_paused());
        assert!(!parent.is_cancelled());
    }
    #[test]
    fn ancestor_cancellation_wakes_paused_worker_and_linked_controls_survive() {
        let parent = Cancellation::default();
        let job = parent.pausable_child();
        let linked = Cancellation::default().linked(&job);
        job.set_pause_requested(true);
        let (sent, received) = mpsc::channel();
        let thread = thread::spawn(move || sent.send(linked.wait_if_paused()).unwrap());
        await_paused(&job);
        parent.cancel();
        assert!(!received.recv_timeout(Duration::from_secs(2)).unwrap());
        thread.join().unwrap();
        assert!(!job.is_paused());
        assert!(!job.pause_requested());
    }
}
