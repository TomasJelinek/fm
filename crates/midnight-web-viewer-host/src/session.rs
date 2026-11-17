use crate::{Provider, Resources, api};
use midnight_plugin_host::CapturedFile;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread::JoinHandle,
};
#[path = "session_actor.rs"]
mod actor;

#[derive(Default)]
struct Output {
    events: VecDeque<api::Event>,
    frame: Option<api::Event>,
}
impl Output {
    fn put(&mut self, event: api::Event) -> Result<(), String> {
        if matches!(event.kind, api::EventKind::Frame(_)) {
            self.frame = Some(event);
            return Ok(());
        }
        if matches!(event.kind, api::EventKind::State(_)) {
            self.events
                .retain(|old| !matches!(old.kind, api::EventKind::State(_)));
        }
        if self.events.len() >= 64 {
            return Err("Browser event queue exhausted".into());
        }
        self.events.push_back(event);
        Ok(())
    }
    /// Replace pending output with the one diagnostic a stopped session can still deliver.
    fn terminal(&mut self, document: u64, message: String) {
        self.frame = None;
        self.events.clear();
        self.events.push_back(api::Event {
            header: api::Header {
                session: 1,
                document,
                sequence: 1,
            },
            kind: api::EventKind::Error {
                code: api::ErrorCode::Runtime,
                message,
            },
        });
    }
}

/// Bounded nonblocking command endpoint. Construction, destruction and drain belong on the
/// viewer backend; the frontend talks only to its existing ViewerSession queue.
pub struct Session {
    input: Arc<Mutex<midnight_web_viewer_sdk::CommandQueue>>,
    sequence: AtomicU64,
    output: Arc<Mutex<Output>>,
    closed: Arc<AtomicBool>,
    document: Arc<AtomicU64>,
    wake: Arc<dyn Fn() -> bool + Send + Sync>,
    thread: Option<JoinHandle<()>>,
}

impl Session {
    pub fn open(
        provider: Provider,
        source: CapturedFile,
        resources: Option<Arc<dyn Resources>>,
        viewport: api::Viewport,
        wake: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> Result<Self, String> {
        let input = Arc::new(Mutex::new(
            midnight_web_viewer_sdk::CommandQueue::new(64).map_err(|error| error.to_string())?,
        ));
        let receiver = input.clone();
        let output = Arc::new(Mutex::new(Output::default()));
        let closed = Arc::new(AtomicBool::new(false));
        let stopped = closed.clone();
        let document = Arc::new(AtomicU64::new(1));
        let requested_document = document.clone();
        let events = output.clone();
        let actor_wake = wake.clone();
        let resources = resources.or_else(|| {
            source
                .resources()
                .map(|value| Arc::new(CapturedResources(value)) as Arc<dyn Resources>)
        });
        let thread = std::thread::Builder::new()
            .name("viewer-html".into())
            .spawn(move || {
                let result = actor::run(
                    provider,
                    source,
                    resources,
                    viewport,
                    receiver,
                    &events,
                    &stopped,
                    &requested_document,
                    &actor_wake,
                );
                // A requested close needs no diagnostic, and a failed send already published
                // the reason; a transport error caused by that stop must not replace it.
                if let Err(message) = result
                    && !stopped.load(Ordering::Acquire)
                {
                    // Only diagnostics, never document contents or URLs, are sent to presentation.
                    if let Ok(mut output) = events.lock() {
                        output.frame = None;
                        output.events.clear();
                        let _ = output.put(api::Event {
                            header: api::Header {
                                session: 1,
                                document: 1,
                                sequence: 1,
                            },
                            kind: api::EventKind::Error {
                                code: api::ErrorCode::Runtime,
                                message,
                            },
                        });
                    }
                    actor_wake();
                }
                stopped.store(true, Ordering::Release);
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            input,
            sequence: AtomicU64::new(1),
            output,
            closed,
            document,
            wake,
            thread: Some(thread),
        })
    }

    pub fn send(&self, command: api::CommandKind) -> bool {
        if self.closed.load(Ordering::Acquire)
            || matches!(
                command,
                api::CommandKind::Open { .. }
                    | api::CommandKind::ResourceReply(_)
                    | api::CommandKind::FrameAck { .. }
            )
        {
            return false;
        }
        if matches!(command, api::CommandKind::Policy(_)) {
            if self
                .document
                .try_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                    value.checked_add(1)
                })
                .is_err()
            {
                self.fail("Browser document IDs are exhausted; reopen the document.");
                return false;
            }
            if let Ok(mut output) = self.output.lock() {
                output.frame = None;
                output.events.clear();
            }
        }
        let sequence = self
            .sequence
            .try_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1)
            });
        let accepted = sequence.is_ok_and(|sequence| {
            self.input.lock().is_ok_and(|mut queue| {
                queue
                    .push(api::Command {
                        header: api::Header {
                            session: 1,
                            document: self.document.load(Ordering::Acquire),
                            sequence,
                        },
                        kind: command,
                    })
                    .is_ok()
            })
        });
        if !accepted {
            // Never lose ordered button/key releases and leave a page with stuck input:
            // the browser stops, and presentation is told why instead of retrying it.
            self.fail("Browser input overflowed while the page was busy; reopen the document.");
            return false;
        }
        true
    }

    /// Stop the session with a terminal diagnostic the presentation drains like any error.
    fn fail(&self, message: &str) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Ok(mut output) = self.output.lock() {
            output.terminal(self.document.load(Ordering::Acquire), message.into());
        }
        (self.wake)();
    }

    pub fn drain(&self, maximum: usize) -> Vec<api::Event> {
        let Ok(mut output) = self.output.lock() else {
            return Vec::new();
        };
        let document = self.document.load(Ordering::Acquire);
        output.events.retain(|event| {
            event.header.document == document || matches!(event.kind, api::EventKind::Error { .. })
        });
        if output
            .frame
            .as_ref()
            .is_some_and(|frame| frame.header.document != document)
        {
            output.frame = None;
        }
        let count = maximum.min(output.events.len());
        let mut events: Vec<_> = output.events.drain(..count).collect();
        if events.len() < maximum
            && let Some(frame) = output.frame.take()
        {
            events.push(frame);
        }
        events
    }
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }
    pub fn finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.close();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Debug)]
struct CapturedResources(Arc<dyn midnight_plugin_host::ContentResources>);
impl Resources for CapturedResources {
    fn capture(&self, segments: &[String]) -> Result<CapturedFile, String> {
        self.0.capture(segments).map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(sequence: u64, kind: api::EventKind) -> api::Event {
        api::Event {
            header: api::Header {
                session: 1,
                document: 1,
                sequence,
            },
            kind,
        }
    }
    #[test]
    fn frame_mailbox_retains_only_latest_owned_pixels() {
        let mut output = Output::default();
        for sequence in 1..=300 {
            output
                .put(event(
                    sequence,
                    api::EventKind::Frame(api::Frame {
                        viewport_revision: 1,
                        acknowledged_input: 1,
                        width: 1,
                        height: 1,
                        rgba: vec![0; 4],
                    }),
                ))
                .unwrap();
        }
        assert_eq!(output.frame.as_ref().unwrap().header.sequence, 300);
        assert!(output.events.is_empty());
    }
    #[test]
    fn input_overflow_stops_the_session_with_a_drained_terminal_error() {
        let woken = Arc::new(AtomicU64::new(0));
        let count = woken.clone();
        let output = Arc::new(Mutex::new(Output::default()));
        output
            .lock()
            .unwrap()
            .put(event(1, api::EventKind::Ready))
            .unwrap();
        let session = Session {
            input: Arc::new(Mutex::new(
                midnight_web_viewer_sdk::CommandQueue::new(2).unwrap(),
            )),
            sequence: AtomicU64::new(1),
            output,
            closed: Arc::new(AtomicBool::new(false)),
            document: Arc::new(AtomicU64::new(1)),
            wake: Arc::new(move || {
                count.fetch_add(1, Ordering::SeqCst);
                true
            }),
            thread: None,
        };
        assert!(session.send(api::CommandKind::Copy));
        assert!(session.send(api::CommandKind::Copy));
        assert!(!session.send(api::CommandKind::Copy), "the queue is full");
        assert!(session.closed.load(Ordering::Acquire));
        assert_eq!(woken.load(Ordering::SeqCst), 1);
        let events = session.drain(8);
        assert!(
            matches!(&events[..], [api::Event { kind: api::EventKind::Error { message, .. }, .. }]
                if message.contains("overflowed")),
            "a silently closed browser would keep reporting a transient busy queue"
        );
        assert!(!session.send(api::CommandKind::Copy));
        assert_eq!(
            woken.load(Ordering::SeqCst),
            1,
            "the failure is reported once"
        );
    }

    #[test]
    fn control_events_are_bounded_and_not_silently_dropped() {
        let mut output = Output::default();
        for sequence in 1..=64 {
            output.put(event(sequence, api::EventKind::Ready)).unwrap();
        }
        assert!(output.put(event(65, api::EventKind::Closed)).is_err());
        assert!(
            output
                .put(event(
                    66,
                    api::EventKind::State(api::State {
                        title: String::new(),
                        url: String::new(),
                        loading: false,
                        editable: false,
                        zoom: 1.0,
                    }),
                ))
                .is_err()
        );
        assert_eq!(output.events.len(), 64);
    }
}
