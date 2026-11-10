//! Latest-frame output with one coalesced event-loop wake, never one task per raster.
use midnight_core::ViewerOutput;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

type Notify = Arc<dyn Fn() -> bool + Send + Sync>;

#[derive(Default)]
pub struct Mailbox {
    output: Mutex<Vec<ViewerOutput>>,
    opening: Mutex<Opening>,
    notify: Mutex<Option<Notify>>,
    wake_pending: AtomicBool,
}

#[derive(Default)]
struct Opening {
    finished: bool,
    result: Option<Result<(), String>>,
}

impl Mailbox {
    /// Consume the initial backend content-ready or failure acknowledgement once.
    pub fn take_open_result(&self) -> Option<Result<(), String>> {
        self.opening
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .result
            .take()
    }

    /// Resolve once; the caller queues the accompanying frame/status to wake the UI.
    pub(crate) fn finish_open(&self, result: Result<(), String>) {
        let mut opening = self.opening.lock().unwrap_or_else(|e| e.into_inner());
        if !opening.finished {
            opening.finished = true;
            opening.result = Some(result);
        }
    }

    /// Register an event-loop wake. Return true only after queueing the wake;
    /// false leaves the next output able to retry after event-loop shutdown/error.
    pub fn notify_on_ready(&self, notify: Notify) {
        *self.notify.lock().unwrap_or_else(|e| e.into_inner()) = Some(notify);
        let ready = !self
            .output
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty();
        if ready {
            self.wake();
        }
    }

    pub fn take(&self) -> Vec<ViewerOutput> {
        let mut pending = self.output.lock().unwrap_or_else(|e| e.into_inner());
        self.wake_pending.store(false, Ordering::Release);
        std::mem::take(&mut *pending)
    }

    pub(crate) fn put(&self, output: ViewerOutput) {
        let mut pending = self.output.lock().unwrap_or_else(|e| e.into_inner());
        match &output {
            ViewerOutput::Frame(_) => pending.retain(|old| !matches!(old, ViewerOutput::Frame(_))),
            ViewerOutput::Status(_) => {
                pending.retain(|old| !matches!(old, ViewerOutput::Status(_)))
            }
            ViewerOutput::ImageUnavailable { .. } => {
                pending.retain(|old| !matches!(old, ViewerOutput::ImageUnavailable { .. }))
            }
            ViewerOutput::Closed => pending.clear(),
            ViewerOutput::CopyText(_) => {
                pending.retain(|old| !matches!(old, ViewerOutput::CopyText(_)))
            }
            ViewerOutput::WebShortcut(_) => {}
            ViewerOutput::WebState { .. } => {
                pending.retain(|old| !matches!(old, ViewerOutput::WebState { .. }))
            }
        }
        if pending.len() < 8 {
            match &output {
                ViewerOutput::Frame(_) => self.finish_open(Ok(())),
                ViewerOutput::ImageUnavailable { message, .. } => {
                    self.finish_open(Err(message.clone()))
                }
                ViewerOutput::Closed => {
                    self.finish_open(Err("Viewer closed before displaying content.".into()))
                }
                _ => {}
            }
            pending.push(output);
        }
        drop(pending);
        self.wake();
    }

    fn wake(&self) {
        let notify = self
            .notify
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(notify) = notify
            && !self.wake_pending.swap(true, Ordering::AcqRel)
            && !notify()
        {
            self.wake_pending.store(false, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn late_subscriber_and_output_bursts_wake_once_until_the_ui_drains() {
        let mailbox = Mailbox::default();
        mailbox.put(ViewerOutput::Status("already ready".into()));
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        mailbox.notify_on_ready(Arc::new(move || {
            observed.fetch_add(1, Ordering::Relaxed);
            true
        }));
        assert_eq!(
            calls.load(Ordering::Relaxed),
            1,
            "late subscriber must not wait for polling"
        );
        for number in 0..1000 {
            mailbox.put(ViewerOutput::Status(number.to_string()));
        }
        assert_eq!(calls.load(Ordering::Relaxed), 1, "bounded event-loop queue");
        assert!(
            matches!(mailbox.take().as_slice(), [ViewerOutput::Status(value)] if value == "999")
        );
        mailbox.put(ViewerOutput::Closed);
        assert_eq!(calls.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn rejected_wake_keeps_latest_output_and_allows_the_next_output_to_retry() {
        let mailbox = Mailbox::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        mailbox.notify_on_ready(Arc::new(move || {
            observed.fetch_add(1, Ordering::Relaxed);
            false
        }));
        mailbox.put(ViewerOutput::Status("one".into()));
        mailbox.put(ViewerOutput::Status("two".into()));
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        assert!(
            matches!(mailbox.take().as_slice(), [ViewerOutput::Status(value)] if value == "two")
        );
    }
}
