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
    /// End the output of a session the browser closed: the queued events (an error among
    /// them stays ahead), then the newest pending frame, then the protocol's Closed event.
    /// That is the session's last output, so it needs no room in the queue's bound.
    fn closed(&mut self, document: u64) {
        self.events.extend(self.frame.take());
        self.events.push_back(api::Event {
            header: api::Header {
                session: 1,
                document,
                sequence: 1,
            },
            kind: api::EventKind::Closed,
        });
    }
}

/// Publish how the actor stopped and wake the consumer before marking the session stopped,
/// so the end is drainable once `send` refuses commands. A close the consumer requested
/// publishes nothing, and neither does a stop after a failed send, which already published
/// its reason.
fn finish(
    result: Result<actor::End, String>,
    output: &Mutex<Output>,
    stopped: &AtomicBool,
    document: &AtomicU64,
    wake: &dyn Fn() -> bool,
) {
    if !matches!(result, Ok(actor::End::Requested)) && !stopped.load(Ordering::Acquire) {
        if let Ok(mut output) = output.lock() {
            let document = document.load(Ordering::Acquire);
            match result {
                // Only diagnostics, never document contents or URLs, reach presentation.
                Err(message) => output.terminal(document, message),
                // An idle consumer learns of the closure without sending another command.
                Ok(_) => output.closed(document),
            }
        }
        wake();
    }
    stopped.store(true, Ordering::Release);
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
                finish(result, &events, &stopped, &requested_document, &*actor_wake);
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
                // A browser that closed since the check above ended the session, not only
                // the replaced document: its Closed event stays.
                output
                    .events
                    .retain(|event| matches!(event.kind, api::EventKind::Closed));
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

    /// Take queued output in order, then the newest frame. When the session stops for a
    /// failure, one terminal error replaces its pending output; when the browser closes it,
    /// the protocol's Closed event follows that output. Either is published, and the wake
    /// called, before `send` refuses commands. A close the consumer requests publishes
    /// nothing.
    pub fn drain(&self, maximum: usize) -> Vec<api::Event> {
        let Ok(mut output) = self.output.lock() else {
            return Vec::new();
        };
        let document = self.document.load(Ordering::Acquire);
        // The session's end, like an error, outlives a replaced document.
        output.events.retain(|event| {
            event.header.document == document
                || matches!(
                    event.kind,
                    api::EventKind::Error { .. } | api::EventKind::Closed
                )
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
    fn frame(sequence: u64) -> api::Event {
        event(
            sequence,
            api::EventKind::Frame(api::Frame {
                viewport_revision: 1,
                acknowledged_input: 1,
                width: 1,
                height: 1,
                rgba: vec![0; 4],
            }),
        )
    }
    /// A session without an actor thread: `end` plays how its actor stops, and `woken`
    /// counts its wakes.
    fn idle(woken: &Arc<AtomicU64>, capacity: usize) -> Session {
        let count = woken.clone();
        Session {
            input: Arc::new(Mutex::new(
                midnight_web_viewer_sdk::CommandQueue::new(capacity).unwrap(),
            )),
            sequence: AtomicU64::new(1),
            output: Arc::new(Mutex::new(Output::default())),
            closed: Arc::new(AtomicBool::new(false)),
            document: Arc::new(AtomicU64::new(1)),
            wake: Arc::new(move || {
                count.fetch_add(1, Ordering::SeqCst);
                true
            }),
            thread: None,
        }
    }
    fn end(session: &Session, result: Result<actor::End, String>) {
        finish(
            result,
            &session.output,
            &session.closed,
            &session.document,
            &*session.wake,
        );
    }
    fn pending(session: &Session, events: impl IntoIterator<Item = api::Event>) {
        let mut output = session.output.lock().unwrap();
        for event in events {
            output.put(event).unwrap();
        }
    }
    fn kinds(session: &Session) -> Vec<api::EventKind> {
        session
            .drain(64)
            .into_iter()
            .map(|event| event.kind)
            .collect()
    }
    fn runtime(message: &str) -> api::EventKind {
        api::EventKind::Error {
            code: api::ErrorCode::Runtime,
            message: message.into(),
        }
    }
    #[test]
    fn frame_mailbox_retains_only_latest_owned_pixels() {
        let mut output = Output::default();
        for sequence in 1..=300 {
            output.put(frame(sequence)).unwrap();
        }
        assert_eq!(output.frame.as_ref().unwrap().header.sequence, 300);
        assert!(output.events.is_empty());
    }
    #[test]
    fn a_browser_that_closes_itself_ends_the_output_once_and_wakes_the_consumer() {
        let woken = Arc::new(AtomicU64::new(0));
        let session = idle(&woken, 64);
        pending(&session, [event(1, api::EventKind::Ready), frame(2)]);
        end(&session, Ok(actor::End::Browser));
        assert_eq!(
            woken.load(Ordering::SeqCst),
            1,
            "an idle consumer learns of it"
        );
        assert!(!session.send(api::CommandKind::Copy), "the session stopped");
        assert_eq!(
            kinds(&session),
            [api::EventKind::Ready, frame(2).kind, api::EventKind::Closed],
            "pending output, its newest frame included, precedes the end"
        );
        assert!(kinds(&session).is_empty(), "the end is published once");
        assert_eq!(woken.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn a_browser_error_stays_ahead_of_the_closure_it_caused() {
        let woken = Arc::new(AtomicU64::new(0));
        let session = idle(&woken, 64);
        let error = runtime("The document could not be opened");
        pending(&session, [event(1, error.clone())]);
        end(&session, Ok(actor::End::Browser));
        assert_eq!(kinds(&session), [error, api::EventKind::Closed]);
    }
    #[test]
    fn a_session_the_consumer_ends_publishes_no_closed_event() {
        for (close, result) in [
            (true, Ok(actor::End::Requested)),
            // The consumer sent Close instead.
            (false, Ok(actor::End::Requested)),
            // The browser closed itself, or failed, while the consumer closed the session.
            (true, Ok(actor::End::Browser)),
            (true, Err("Chromium transport stopped".to_string())),
        ] {
            let woken = Arc::new(AtomicU64::new(0));
            let session = idle(&woken, 64);
            pending(&session, [event(1, api::EventKind::Ready)]);
            if close {
                session.close();
            } else {
                assert!(session.send(api::CommandKind::Close));
            }
            end(&session, result);
            assert_eq!(woken.load(Ordering::SeqCst), 0, "nothing new to drain");
            assert_eq!(kinds(&session), [api::EventKind::Ready]);
            assert!(!session.send(api::CommandKind::Copy));
        }
    }
    #[test]
    fn a_policy_sent_while_the_browser_closes_keeps_the_end() {
        let woken = Arc::new(AtomicU64::new(0));
        let session = idle(&woken, 64);
        // The actor published the end but has not marked the session stopped yet.
        session.output.lock().unwrap().closed(1);
        assert!(session.send(api::CommandKind::Policy(api::Policy::default())));
        assert_eq!(
            kinds(&session),
            [api::EventKind::Closed],
            "the replacement document's consumer still learns of the end"
        );
    }
    #[test]
    fn an_actor_failure_replaces_pending_output_with_its_reason() {
        let woken = Arc::new(AtomicU64::new(0));
        let session = idle(&woken, 64);
        pending(&session, [event(1, api::EventKind::Ready), frame(2)]);
        end(&session, Err("Chromium disconnected".into()));
        assert_eq!(woken.load(Ordering::SeqCst), 1);
        assert!(!session.send(api::CommandKind::Copy));
        assert_eq!(kinds(&session), [runtime("Chromium disconnected")]);
    }
    #[test]
    fn input_overflow_stops_the_session_with_a_drained_terminal_error() {
        let woken = Arc::new(AtomicU64::new(0));
        let session = idle(&woken, 2);
        pending(&session, [event(1, api::EventKind::Ready)]);
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
        // The actor stops after the failure, even if its browser closed itself meanwhile.
        end(&session, Ok(actor::End::Browser));
        assert!(kinds(&session).is_empty(), "no Closed follows the reason");
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
