//! Bounded browser output: controls retain FIFO ordering and paints retain only the latest frame.
use midnight_web_viewer_api as api;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

pub(crate) struct Output {
    controls: mpsc::SyncSender<api::Event>,
    wake: mpsc::SyncSender<()>,
    pub(crate) frame: Mutex<Option<api::Event>>,
    pub(crate) failed: AtomicBool,
}
pub(crate) struct Receiver {
    pub(crate) controls: mpsc::Receiver<api::Event>,
    wake: mpsc::Receiver<()>,
    consecutive_controls: usize,
}
pub(crate) fn channel() -> (Arc<Output>, Receiver) {
    let (controls, receive) = mpsc::sync_channel(64);
    let (wake, wakeups) = mpsc::sync_channel(1);
    (
        Arc::new(Output {
            controls,
            wake,
            frame: Mutex::new(None),
            failed: AtomicBool::new(false),
        }),
        Receiver {
            controls: receive,
            wake: wakeups,
            consecutive_controls: 0,
        },
    )
}
impl Output {
    pub(crate) fn emit(&self, header: api::Header, kind: api::EventKind) {
        let event = api::Event { header, kind };
        if matches!(event.kind, api::EventKind::Frame(_)) {
            if let Ok(mut frame) = self.frame.lock() {
                #[cfg(feature = "diagnostic-timing")]
                crate::trace::record(crate::trace::Event::FrameQueued, event.header.document, 0);
                *frame = Some(event);
            }
        } else if self.controls.try_send(event).is_err() {
            self.failed.store(true, Ordering::Release);
        }
        // A single retained wake is enough: the reader always rechecks both queues.
        // Full means a wake is already pending; no frame/control storage is added.
        let _ = self.wake.try_send(());
    }
}
impl Receiver {
    pub(crate) fn next(&mut self, output: &Output) -> Option<api::Event> {
        loop {
            // Resource/state traffic must not starve an already painted viewport.
            // Controls preserve their FIFO order and remain limited to 64 items.
            if self.consecutive_controls >= 8 {
                self.consecutive_controls = 0;
                if let Some(frame) = output.frame.lock().ok().and_then(|mut f| f.take()) {
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
            if let Some(frame) = output.frame.lock().ok().and_then(|mut f| f.take()) {
                self.consecutive_controls = 0;
                return Some(frame);
            }
            // Sleeping has no polling latency. An event arriving between the empty
            // checks and this receive leaves a token, so no notification is lost.
            self.wake.recv().ok()?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn header(sequence: u64) -> api::Header {
        api::Header {
            session: 1,
            document: 1,
            sequence,
        }
    }
    fn paint(value: u8) -> api::EventKind {
        api::EventKind::Frame(api::Frame {
            viewport_revision: 1,
            acknowledged_input: 0,
            width: 1,
            height: 1,
            rgba: vec![value; 4],
        })
    }
    #[test]
    fn queued_paint_is_published_during_continuous_control_traffic() {
        let (output, mut receiver) = channel();
        output.emit(header(1), paint(7));
        for sequence in 2..=65 {
            output.emit(header(sequence), api::EventKind::Ready);
        }
        for sequence in 2..=9 {
            assert_eq!(receiver.next(&output).unwrap().header.sequence, sequence);
        }
        assert!(
            matches!(
                receiver.next(&output).unwrap().kind,
                api::EventKind::Frame(_)
            ),
            "a ready paint must not wait for control traffic to become idle"
        );
        assert_eq!(receiver.next(&output).unwrap().header.sequence, 10);
    }
    #[test]
    fn coalesced_wake_tokens_do_not_lose_later_paints() {
        let (output, mut receiver) = channel();
        let (observed, complete) = mpsc::sync_channel(1);
        let (next, pending) = mpsc::sync_channel(1);
        let reader_output = output.clone();
        let reader = std::thread::spawn(move || {
            for _ in 0..100 {
                let event = receiver.next(&reader_output).unwrap();
                observed.send(event).unwrap();
                pending.recv().unwrap();
            }
        });
        for value in 1..=100 {
            output.emit(header(value.into()), paint(value));
            let event = complete
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            let api::EventKind::Frame(frame) = event.kind else {
                panic!("paint")
            };
            assert_eq!(frame.rgba, [value; 4]);
            next.send(()).unwrap();
        }
        reader.join().unwrap();
    }
    #[test]
    fn frame_coalescing_and_control_overflow_keep_existing_bounds() {
        let (output, mut receiver) = channel();
        for value in 1..=100 {
            output.emit(header(value.into()), paint(value));
        }
        let api::EventKind::Frame(frame) = receiver.next(&output).unwrap().kind else {
            panic!("frame")
        };
        assert_eq!(frame.rgba, [100; 4]);
        for sequence in 101..165 {
            output.emit(header(sequence), api::EventKind::Ready);
        }
        assert!(!output.failed.load(Ordering::Acquire));
        output.emit(header(165), api::EventKind::Ready);
        assert!(output.failed.load(Ordering::Acquire));
    }
}
