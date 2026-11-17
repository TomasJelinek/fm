//! Bounded browser output: controls retain FIFO ordering, paints and browser state
//! retain only their latest value, and resource releases wait in their own lane.
use midnight_web_viewer_api as api;
use std::collections::VecDeque;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

/// FIFO control capacity. Host resource requests are separately limited to
/// `MAX_IN_FLIGHT` (32), so they alone can never fill it.
pub(crate) const CONTROL_CAPACITY: usize = 256;
/// Pending `ResourceRelease` events. Every one returns a distinct live host
/// capability (the host keeps at most 2048), so a conforming host cannot fill it.
pub(crate) const RELEASE_CAPACITY: usize = 4096;

pub(crate) struct Output {
    controls: mpsc::SyncSender<api::Event>,
    wake: mpsc::SyncSender<()>,
    pub(crate) frame: Mutex<Option<api::Event>>,
    state: Mutex<Option<api::Event>>,
    releases: Mutex<VecDeque<api::Event>>,
    pub(crate) failed: AtomicBool,
    finished: AtomicBool,
}
pub(crate) struct Receiver {
    pub(crate) controls: mpsc::Receiver<api::Event>,
    wake: mpsc::Receiver<()>,
    consecutive_controls: usize,
}
pub(crate) fn channel() -> (Arc<Output>, Receiver) {
    let (controls, receive) = mpsc::sync_channel(CONTROL_CAPACITY);
    let (wake, wakeups) = mpsc::sync_channel(1);
    (
        Arc::new(Output {
            controls,
            wake,
            frame: Mutex::new(None),
            state: Mutex::new(None),
            releases: Mutex::new(VecDeque::new()),
            failed: AtomicBool::new(false),
            finished: AtomicBool::new(false),
        }),
        Receiver {
            controls: receive,
            wake: wakeups,
            consecutive_controls: 0,
        },
    )
}
fn take(slot: &Mutex<Option<api::Event>>) -> Option<api::Event> {
    slot.lock().ok().and_then(|mut value| value.take())
}
impl Output {
    pub(crate) fn emit(&self, header: api::Header, kind: api::EventKind) {
        // Provider events are validated before queueing (without copying frame pixels),
        // so the writer can only fail on a broken pipe. An out-of-budget event becomes a
        // bounded error instead of terminating the transport.
        let message = api::Message::Event(api::Event { header, kind });
        let valid = message.validate().is_ok();
        let api::Message::Event(mut event) = message else {
            return;
        };
        if !valid {
            event.kind = api::EventKind::Error {
                code: api::ErrorCode::Runtime,
                message: "Browser event exceeded protocol limits".into(),
            };
        }
        match event.kind {
            api::EventKind::Frame(_) => {
                if let Ok(mut frame) = self.frame.lock() {
                    #[cfg(feature = "diagnostic-timing")]
                    crate::trace::record(
                        crate::trace::Event::FrameQueued,
                        event.header.document,
                        0,
                    );
                    *frame = Some(event);
                }
            }
            // Only the newest title/loading/zoom state matters; a title-changing script
            // cannot flood the bounded control queue.
            api::EventKind::State(_) => {
                if let Ok(mut state) = self.state.lock() {
                    *state = Some(event);
                }
            }
            api::EventKind::ResourceRelease { .. } => {
                let queued = self.releases.lock().is_ok_and(|mut releases| {
                    releases.len() < RELEASE_CAPACITY && {
                        releases.push_back(event);
                        true
                    }
                });
                if !queued {
                    self.failed.store(true, Ordering::Release);
                }
            }
            _ => {
                if self.controls.try_send(event).is_err() {
                    self.failed.store(true, Ordering::Release);
                }
            }
        }
        // A single retained wake is enough: the reader always rechecks every queue.
        // Full means a wake is already pending; no frame/control storage is added.
        let _ = self.wake.try_send(());
    }
    /// Drops the presentation of a retired document (its paint and state).
    pub(crate) fn discard_presentation(&self) {
        for slot in [&self.frame, &self.state] {
            if let Ok(mut value) = slot.lock() {
                *value = None;
            }
        }
    }
    /// Lets the reader return `None` once everything queued has been written. The
    /// reader's own `Output` keeps the senders alive, so disconnection never ends it.
    pub(crate) fn finish(&self) {
        self.finished.store(true, Ordering::Release);
        let _ = self.wake.try_send(());
    }
}
impl Receiver {
    pub(crate) fn next(&mut self, output: &Output) -> Option<api::Event> {
        loop {
            // Resource/state traffic must not starve an already painted viewport.
            if self.consecutive_controls >= 8 {
                self.consecutive_controls = 0;
                if let Some(frame) = take(&output.frame) {
                    return Some(frame);
                }
            }
            match self.controls.try_recv() {
                Ok(event) => {
                    self.consecutive_controls += 1;
                    return Some(event);
                }
                Err(mpsc::TryRecvError::Disconnected) => return None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
            // Releases follow every earlier FIFO control, including reads of the same
            // resource, and are written before newer frames once controls are idle.
            let release = output
                .releases
                .lock()
                .ok()
                .and_then(|mut releases| releases.pop_front());
            if let Some(event) = release.or_else(|| take(&output.state)) {
                self.consecutive_controls += 1;
                return Some(event);
            }
            if let Some(frame) = take(&output.frame) {
                self.consecutive_controls = 0;
                return Some(frame);
            }
            if output.finished.load(Ordering::Acquire) {
                return None;
            }
            // Sleeping has no polling latency. An event arriving between the empty
            // checks and this receive leaves a token, so no notification is lost.
            self.wake.recv().ok()?;
        }
    }
}

#[cfg(test)]
#[path = "output_tests.rs"]
mod tests;
