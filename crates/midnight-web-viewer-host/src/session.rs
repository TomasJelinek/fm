use crate::resource_jobs::{CleanupStatus, Generation, SourceOwner};
use crate::{Provider, Resources, api};
use midnight_plugin_host::CapturedFile;
use std::{
    collections::VecDeque,
    panic::AssertUnwindSafe,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::JoinHandle,
};
#[path = "session_actor.rs"]
mod actor;

/// Consumer commands the session queues for its actor.
const QUEUED_COMMANDS: usize = 64;

/// Immutable host ceiling. A renderer or injected command cannot raise this permission.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PolicyCeiling {
    #[default]
    UserChoice,
    OfflineOnly,
}

#[derive(Default)]
struct Output {
    events: VecDeque<api::Event>,
    frame: Option<api::Event>,
    /// The session's end, a terminal error or the protocol's Closed event, is published.
    /// The first end stays the last output: nothing follows or replaces it.
    ended: bool,
}
impl Output {
    fn put_source_changed(&mut self, origin: api::Event) -> Result<(), String> {
        if self.ended {
            return Ok(());
        }
        if self
            .events
            .len()
            .checked_add(2)
            .is_none_or(|length| length > 64)
        {
            return Err("Browser event queue exhausted".into());
        }
        let header = origin.header;
        // The header is resource-request provenance. Its asynchronous denial may
        // finish after newer output was queued; retain main's queued header order.
        let key = (header.document, header.sequence);
        let at = self
            .events
            .partition_point(|event| (event.header.document, event.header.sequence) <= key);
        self.events.insert(at, origin);
        self.events.insert(
            at + 1,
            api::Event {
                header,
                kind: api::EventKind::Error {
                    code: api::ErrorCode::ResourceChanged,
                    message: "Source changed; showing previous contents. Reopen to refresh.".into(),
                },
            },
        );
        Ok(())
    }

    fn put(&mut self, event: api::Event) -> Result<(), String> {
        // The actor can still receive output after a failed send stopped the session.
        if self.ended {
            return Ok(());
        }
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
    /// End the session with the one diagnostic a stopped session can still deliver, which
    /// replaces pending output. False when the session already ended.
    fn terminal(&mut self, document: u64, message: String) -> bool {
        if std::mem::replace(&mut self.ended, true) {
            return false;
        }
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
        true
    }
    /// Where the retained frame belongs among the queued events in the browser's order
    /// (document, then sequence). The queue keeps that order: the actor queues each
    /// document's events in sequence and documents in order, a replacing State is the
    /// newest event, and a Policy change clears the queue.
    fn frame_position(&self) -> Option<usize> {
        let frame = self.frame.as_ref()?;
        let order = |event: &api::Event| (event.header.document, event.header.sequence);
        Some(
            self.events
                .partition_point(|event| order(event) < order(frame)),
        )
    }
    /// End the output of a session the browser closed with the protocol's Closed event. The
    /// output the browser sent before it keeps the browser's order, the newest frame
    /// included, so an error sent after that frame stays the newest output (and the reason).
    /// That is the session's last output, so it needs no room in the queue's bound. False
    /// when the session already ended.
    fn closed(&mut self, document: u64) -> bool {
        if std::mem::replace(&mut self.ended, true) {
            return false;
        }
        if let Some(at) = self.frame_position()
            && let Some(frame) = self.frame.take()
        {
            self.events.insert(at, frame);
        }
        self.events.push_back(api::Event {
            header: api::Header {
                session: 1,
                document,
                sequence: 1,
            },
            kind: api::EventKind::Closed,
        });
        true
    }
}

/// The actor thread: run the actor, then publish how it stopped and wake the consumer
/// before marking the session stopped, so the end is drainable once `send` refuses
/// commands. A panic is a failure like any other. A close the consumer requested publishes
/// nothing, and neither does a stop after a failed send, which already published its
/// reason.
fn supervise(
    actor: impl FnOnce() -> Result<actor::End, String>,
    output: &Mutex<Output>,
    stopped: &AtomicBool,
    document: &AtomicU64,
    wake: &dyn Fn() -> bool,
) {
    let result = std::panic::catch_unwind(AssertUnwindSafe(actor)).unwrap_or_else(|panic| {
        std::mem::forget(panic);
        Err("Browser session failed; reopen the document.".into())
    });
    if !matches!(result, Ok(actor::End::Requested))
        && !stopped.load(Ordering::Acquire)
        && output.lock().is_ok_and(|mut output| {
            let document = document.load(Ordering::Acquire);
            match result {
                // Only diagnostics, never document contents or URLs, reach presentation.
                Err(message) => output.terminal(document, message),
                // An idle consumer learns of the closure without sending another command.
                Ok(_) => output.closed(document),
            }
        })
    {
        wake();
    }
    stopped.store(true, Ordering::Release);
}

/// Run `actor` under `supervise` on the session's thread. Owning `shared` and `actor`, it
/// drops every handle the thread holds when it returns, before `Exit` publishes the exit.
fn supervised(shared: Shared, actor: impl FnOnce(&Shared) -> Result<actor::End, String>) {
    supervise(
        || {
            let result = std::panic::catch_unwind(AssertUnwindSafe(|| actor(&shared)))
                .unwrap_or_else(|panic| {
                    std::mem::forget(panic);
                    let message = "Browser actor panicked; cleanup is unconfirmed".to_owned();
                    *shared
                        .cleanup
                        .lock()
                        .unwrap_or_else(|error| error.into_inner()) = Some(message.clone());
                    Err(message)
                });
            let cleanup = shared
                .cleanup
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone();
            let receipt = cleanup.map_or(Ok(()), Err);
            *shared
                .terminal
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(receipt.clone());
            // Keep terminal publication ahead of the stopped flag, as in the main actor.
            receipt.and(result)
        },
        &shared.output,
        &shared.closed,
        &shared.document,
        &*shared.wake,
    );
    shared.generation.close();
    shared.signal.interrupt();
}

/// What a session shares with its actor thread.
#[derive(Clone)]
struct Shared {
    signal: Arc<actor::Signal>,
    output: Arc<Mutex<Output>>,
    /// The session was closed or stopped: `send` refuses commands.
    closed: Arc<AtomicBool>,
    /// The newest document the consumer requested.
    document: Arc<AtomicU64>,
    wake: Arc<dyn Fn() -> bool + Send + Sync>,
    generation: Arc<Generation>,
    ceiling: PolicyCeiling,
    cleanup: CleanupStatus,
    terminal: Arc<Mutex<Option<Result<(), String>>>>,
}

/// Publishes the actor thread's exit, then wakes the consumer. Created first on that
/// thread, it is dropped last, also when the thread unwinds, so `finished` is true when
/// the consumer handles this wake.
struct Exit {
    exited: Arc<AtomicBool>,
    wake: Arc<dyn Fn() -> bool + Send + Sync>,
}
impl Drop for Exit {
    fn drop(&mut self) {
        self.exited.store(true, Ordering::Release);
        (self.wake)();
    }
}

/// Bounded nonblocking command endpoint. Construction, destruction and drain belong on the
/// viewer backend; the frontend talks only to its existing ViewerSession queue.
pub struct Session {
    signal: Arc<actor::Signal>,
    admission: Mutex<()>,
    sequence: AtomicU64,
    output: Arc<Mutex<Output>>,
    closed: Arc<AtomicBool>,
    document: Arc<AtomicU64>,
    wake: Arc<dyn Fn() -> bool + Send + Sync>,
    /// The actor thread published its exit (see `Exit`).
    exited: Arc<AtomicBool>,
    generation: Arc<Generation>,
    ceiling: PolicyCeiling,
    terminal: Arc<Mutex<Option<Result<(), String>>>>,
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
        Self::open_with_policy_ceiling(
            provider,
            source,
            resources,
            viewport,
            wake,
            PolicyCeiling::UserChoice,
        )
    }

    pub fn open_with_policy_ceiling(
        provider: Provider,
        source: CapturedFile,
        resources: Option<Arc<dyn Resources>>,
        viewport: api::Viewport,
        wake: Arc<dyn Fn() -> bool + Send + Sync>,
        ceiling: PolicyCeiling,
    ) -> Result<Self, String> {
        let resources = resources.or_else(|| {
            source
                .resources()
                .map(|value| Arc::new(CapturedResources(value)) as Arc<dyn Resources>)
        });
        let cleanup = Arc::new(Mutex::new(None));
        let owner = SourceOwner::new(source, resources, cleanup.clone());
        Self::start(wake, ceiling, cleanup, move |shared| {
            actor::run(provider, owner, viewport, shared)
        })
    }

    /// A session whose actor runs over `transport` instead of a launched browser.
    #[cfg(test)]
    fn with_transport(
        transport: actor::Transport,
        source: CapturedFile,
        resources: Option<Arc<dyn Resources>>,
        viewport: api::Viewport,
        limits: actor::Limits,
        wake: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> Result<Self, String> {
        let cleanup = Arc::new(Mutex::new(None));
        let owner = SourceOwner::new(source, resources, cleanup.clone());
        Self::start(wake, PolicyCeiling::UserChoice, cleanup, move |shared| {
            actor::run_with(transport, owner, viewport, shared, limits)
        })
    }

    /// Run `actor` on the session's thread, supervised (see `supervise`). The thread
    /// publishes its exit and wakes the consumer last (see `Exit`).
    fn start(
        wake: Arc<dyn Fn() -> bool + Send + Sync>,
        ceiling: PolicyCeiling,
        cleanup: CleanupStatus,
        actor: impl FnOnce(&Shared) -> Result<actor::End, String> + Send + 'static,
    ) -> Result<Self, String> {
        let signal = Arc::new(actor::Signal::new(QUEUED_COMMANDS)?);
        let resource_signal = signal.clone();
        let shared = Shared {
            signal,
            output: Arc::default(),
            closed: Arc::default(),
            document: Arc::new(AtomicU64::new(1)),
            wake,
            // Resource-worker completion revokes I/O before supervision publishes the end.
            // Its cancellation flag must not suppress that terminal publication.
            generation: Arc::new(Generation::with_wake(Arc::new(move || {
                resource_signal.resources_ready();
            }))),
            ceiling,
            cleanup,
            terminal: Arc::new(Mutex::new(None)),
        };
        let exited = Arc::new(AtomicBool::new(false));
        let exit = (exited.clone(), shared.wake.clone());
        let thread_shared = shared.clone();
        let thread = std::thread::Builder::new()
            .name("viewer-html".into())
            .spawn(move || {
                let _exit = Exit {
                    exited: exit.0,
                    wake: exit.1,
                };
                supervised(thread_shared, actor);
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            signal: shared.signal,
            admission: Mutex::new(()),
            sequence: AtomicU64::new(1),
            output: shared.output,
            closed: shared.closed,
            document: shared.document,
            wake: shared.wake,
            exited,
            generation: shared.generation,
            ceiling: shared.ceiling,
            terminal: shared.terminal,
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
        if self.ceiling == PolicyCeiling::OfflineOnly
            && matches!(command, api::CommandKind::Policy(_))
        {
            return false;
        }
        // Keep document assignment and queue admission ordered for concurrent callers.
        let Ok(_admission) = self.admission.lock() else {
            self.fail("Browser command admission unavailable; reopen the document.");
            return false;
        };
        if self.closed.load(Ordering::Acquire) {
            return false;
        }
        if matches!(command, api::CommandKind::Policy(_)) {
            // The actor publishes the session's end under this lock, before it marks the
            // session stopped. An end published since the check above stays, and refuses
            // the replacement like a stopped session; a later end names the new document.
            let mut output = self.output.lock();
            if output.as_ref().is_ok_and(|output| output.ended) {
                return false;
            }
            let Some(next) = self.document.load(Ordering::Acquire).checked_add(1) else {
                drop(output);
                self.fail("Browser document IDs are exhausted; reopen the document.");
                return false;
            };
            if self.generation.advance(next).is_err() {
                drop(output);
                self.fail("Browser resource generation was revoked; reopen the document.");
                return false;
            }
            self.document.store(next, Ordering::Release);
            if let Ok(output) = output.as_mut() {
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
            self.signal.push(api::Command {
                header: api::Header {
                    session: 1,
                    document: self.document.load(Ordering::Acquire),
                    sequence,
                },
                kind: command,
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

    /// Stop the session with a terminal diagnostic the presentation drains like any error,
    /// unless the actor already published the session's end, which then stays.
    fn fail(&self, message: &str) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        self.generation.close();
        if self.output.lock().is_ok_and(|mut output| {
            output.terminal(self.document.load(Ordering::Acquire), message.into())
        }) {
            (self.wake)();
        }
        self.signal.interrupt();
    }

    /// Take queued output and the newest frame in the browser's order (document, then
    /// sequence), at most `maximum` events. An error the browser sent after the frame is
    /// drained after it, so it stays the newest output. A frame that would come after the
    /// limit stays for a later drain, where a newer frame can still replace it. The
    /// session's end is its last output, published once, and the wake called, before
    /// `send` refuses commands: when the session stops for a failure (a panic included),
    /// one terminal error replaces its pending output; when the browser closes it, the
    /// protocol's Closed event follows the output the browser sent, in the browser's
    /// order. A close the consumer requests publishes nothing.
    pub fn drain(&self, maximum: usize) -> Vec<api::Event> {
        let Ok(mut output) = self.output.lock() else {
            return Vec::new();
        };
        let document = self.document.load(Ordering::Acquire);
        // A replaced document's output is stale, apart from errors. An ended session's
        // document is never replaced, so its end names the current one.
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
        match output.frame_position() {
            Some(at) if at < maximum => {
                let mut events: Vec<_> = output.events.drain(..at).collect();
                events.extend(output.frame.take());
                let later = (maximum - at - 1).min(output.events.len());
                events.extend(output.events.drain(..later));
                events
            }
            _ => {
                let count = maximum.min(output.events.len());
                output.events.drain(..count).collect()
            }
        }
    }
    /// Stop the session. Its actor wakes at once and ends without publishing anything.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.generation.close();
        self.signal.interrupt();
    }
    /// The actor thread ended its work; dropping the session then only joins the thread's
    /// return. The thread wakes the consumer once this is true.
    pub fn finished(&self) -> bool {
        self.thread.is_none() || self.exited.load(Ordering::Acquire)
    }

    pub fn policy_change_allowed(&self) -> bool {
        self.ceiling == PolicyCeiling::UserChoice
    }

    /// Consume on the viewer backend and qualify all captured-resource cleanup.
    pub fn release_on_worker(mut self) -> Result<(), String> {
        self.finish()
    }

    fn finish(&mut self) -> Result<(), String> {
        self.close();
        if let Some(thread) = self.thread.take()
            && let Err(panic) = thread.join()
        {
            std::mem::forget(panic);
            return Err("Browser actor cleanup panicked".into());
        }
        self.terminal
            .lock()
            .map_err(|_| "Browser completion unavailable")?
            .take()
            .unwrap_or(Ok(()))
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

#[derive(Debug)]
struct CapturedResources(Arc<dyn midnight_plugin_host::ContentResources>);
impl Resources for CapturedResources {
    fn capture(&self, segments: &[String]) -> Result<CapturedFile, String> {
        self.0.capture(segments).map_err(|error| error.to_string())
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), String> {
        match Arc::try_unwrap(self) {
            Ok(CapturedResources(resources)) => resources
                .release_on_worker()
                .map_err(|error| error.to_string()),
            Err(shared) => {
                drop(shared);
                Ok(())
            }
        }
    }
}

#[cfg(test)]
#[path = "session_policy_tests.rs"]
mod policy_tests;

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
    /// A session without an actor thread: `end` plays how its actor stops.
    fn with_wake(
        capacity: usize,
        output: Arc<Mutex<Output>>,
        closed: Arc<AtomicBool>,
        wake: impl Fn() -> bool + Send + Sync + 'static,
    ) -> Session {
        Session {
            signal: Arc::new(actor::Signal::new(capacity).unwrap()),
            admission: Mutex::new(()),
            sequence: AtomicU64::new(1),
            output,
            closed,
            document: Arc::new(AtomicU64::new(1)),
            wake: Arc::new(wake),
            exited: Arc::default(),
            generation: Arc::default(),
            ceiling: PolicyCeiling::UserChoice,
            terminal: Arc::new(Mutex::new(None)),
            thread: None,
        }
    }
    /// A session without an actor thread whose wakes `woken` counts.
    fn idle(woken: &Arc<AtomicU64>, capacity: usize) -> Session {
        let count = woken.clone();
        with_wake(capacity, Arc::default(), Arc::default(), move || {
            count.fetch_add(1, Ordering::SeqCst);
            true
        })
    }
    fn end(session: &Session, result: Result<actor::End, String>) {
        supervise(
            move || result,
            &session.output,
            &session.closed,
            &session.document,
            &*session.wake,
        );
    }
    fn is_runtime_error(events: &[api::Event]) -> bool {
        matches!(
            events,
            [api::Event {
                kind: api::EventKind::Error {
                    code: api::ErrorCode::Runtime,
                    ..
                },
                ..
            }]
        )
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
    fn a_frame_and_a_later_error_drain_in_the_browsers_order() {
        let woken = Arc::new(AtomicU64::new(0));
        let session = idle(&woken, 64);
        let error = runtime("Chromium renderer terminated; reopen the document");
        pending(&session, [frame(1), event(2, error.clone())]);
        assert_eq!(
            kinds(&session),
            [frame(1).kind, error],
            "the error stays the newest output, so the frame's status cannot replace it"
        );
    }
    #[test]
    fn a_frame_older_than_queued_events_is_drained_before_them() {
        let woken = Arc::new(AtomicU64::new(0));
        let session = idle(&woken, 64);
        let found = api::EventKind::FindResult(api::FindResult {
            request_sequence: 2,
            matches: 3,
            active: 1,
            complete: true,
        });
        pending(
            &session,
            [
                event(1, api::EventKind::Ready),
                frame(2),
                event(3, found.clone()),
            ],
        );
        assert_eq!(
            kinds(&session),
            [api::EventKind::Ready, frame(2).kind, found]
        );
    }
    #[test]
    fn a_frame_past_the_drain_limit_stays_replaceable() {
        let woken = Arc::new(AtomicU64::new(0));
        let session = idle(&woken, 64);
        let ready = |sequence| event(sequence, api::EventKind::Ready);
        pending(&session, (1..=8).map(ready).chain([frame(9)]));
        let order = |events: Vec<api::Event>| {
            let frames = events.iter().map(|event| {
                let frame = matches!(event.kind, api::EventKind::Frame(_));
                (event.header.sequence, frame)
            });
            frames.collect::<Vec<_>>()
        };
        assert_eq!(
            order(session.drain(4)),
            [(1, false), (2, false), (3, false), (4, false)],
            "the frame follows events beyond the limit, so it stays"
        );
        pending(&session, [frame(10)]);
        assert_eq!(
            order(session.drain(64)),
            [(5, false), (6, false), (7, false), (8, false), (10, true)],
            "a newer frame replaced the retained one"
        );
    }
    #[test]
    fn an_error_of_a_replaced_document_drains_ahead_of_the_new_documents_output() {
        let woken = Arc::new(AtomicU64::new(0));
        let session = idle(&woken, 64);
        assert!(session.send(api::CommandKind::Policy(api::Policy::default())));
        let current = |mut event: api::Event| {
            event.header.document = 2;
            event
        };
        // The actor admitted this error before the replacement; drain keeps old errors.
        let stale = event(5, runtime("The document could not be opened"));
        let error = runtime("Chromium renderer terminated; reopen the document");
        pending(
            &session,
            [
                stale.clone(),
                current(frame(1)),
                current(event(2, error.clone())),
            ],
        );
        assert_eq!(kinds(&session), [stale.kind, frame(1).kind, error]);
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
        let error = runtime("The document could not be opened");
        for (sent, drained) in [
            (vec![event(1, error.clone())], vec![error.clone()]),
            // The newest frame was painted before the error, so the error stays the newest
            // output and the reason the consumer reports.
            (
                vec![frame(1), event(2, error.clone())],
                vec![frame(1).kind, error.clone()],
            ),
        ] {
            let woken = Arc::new(AtomicU64::new(0));
            let session = idle(&woken, 64);
            pending(&session, sent);
            end(&session, Ok(actor::End::Browser));
            assert_eq!(
                kinds(&session),
                [drained, vec![api::EventKind::Closed]].concat()
            );
        }
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
    fn a_woken_consumer_drains_the_end_before_send_refuses_commands() {
        for (result, published) in [
            (
                Ok(actor::End::Browser),
                vec![api::EventKind::Ready, frame(2).kind, api::EventKind::Closed],
            ),
            (
                Err("Chromium disconnected".to_string()),
                vec![runtime("Chromium disconnected")],
            ),
        ] {
            let output = Arc::new(Mutex::new(Output::default()));
            let closed = Arc::new(AtomicBool::new(false));
            let woken = Arc::new(Mutex::new(Vec::new()));
            // What a consumer woken now finds: whether `send` still accepts commands, and
            // the output it can drain, which it cannot while the lock is still held.
            let wake = {
                let (output, closed, woken) = (output.clone(), closed.clone(), woken.clone());
                move || {
                    let queued = output.try_lock().ok().map(|output| {
                        let events = output.events.iter();
                        events.map(|event| event.kind.clone()).collect::<Vec<_>>()
                    });
                    let accepting = !closed.load(Ordering::Acquire);
                    woken.lock().unwrap().push((accepting, queued));
                    true
                }
            };
            let session = with_wake(64, output, closed, wake);
            pending(&session, [event(1, api::EventKind::Ready), frame(2)]);
            end(&session, result);
            assert_eq!(
                *woken.lock().unwrap(),
                [(true, Some(published))],
                "the end is published before the one wake, which precedes the stop"
            );
            assert!(session.closed.load(Ordering::Acquire));
        }
    }
    #[test]
    fn a_policy_sent_while_the_session_ends_cannot_remove_the_end() {
        for (browser, published) in [
            (true, vec![api::EventKind::Ready, api::EventKind::Closed]),
            (false, vec![runtime("Chromium disconnected")]),
        ] {
            let woken = Arc::new(AtomicU64::new(0));
            let session = idle(&woken, 64);
            pending(&session, [event(1, api::EventKind::Ready)]);
            // The actor published the end and woke the consumer, but has not marked the
            // session stopped yet.
            let mut output = session.output.lock().unwrap();
            assert!(if browser {
                output.closed(1)
            } else {
                output.terminal(1, "Chromium disconnected".into())
            });
            drop(output);
            assert!(
                !session.send(api::CommandKind::Policy(api::Policy::default())),
                "the ended session refuses the replacement"
            );
            assert_eq!(session.document.load(Ordering::Acquire), 1);
            assert_eq!(kinds(&session), published);
        }
    }
    #[test]
    fn the_first_end_published_stays_the_last_output() {
        let woken = Arc::new(AtomicU64::new(0));
        // A failed send stops the session while its actor can still publish browser output.
        let session = idle(&woken, 1);
        assert!(session.send(api::CommandKind::Copy));
        assert!(!session.send(api::CommandKind::Copy), "the queue is full");
        pending(&session, [event(1, api::EventKind::Ready), frame(2)]);
        let events = session.drain(8);
        assert!(
            is_runtime_error(&events),
            "nothing follows, and so hides, the reason: {events:?}"
        );
        assert_eq!(woken.load(Ordering::SeqCst), 1);
        // A send that fails after the browser's end was published, before the session
        // stopped, keeps that end.
        let woken = Arc::new(AtomicU64::new(0));
        let session = idle(&woken, 1);
        assert!(session.output.lock().unwrap().closed(1));
        assert!(session.send(api::CommandKind::Copy));
        assert!(!session.send(api::CommandKind::Copy), "the queue is full");
        assert_eq!(kinds(&session), [api::EventKind::Closed]);
        assert_eq!(
            woken.load(Ordering::SeqCst),
            0,
            "only the actor that published the end wakes the consumer"
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
    fn a_panicking_actor_still_ends_the_session_with_a_failure() {
        let woken = Arc::new(AtomicU64::new(0));
        let session = idle(&woken, 64);
        pending(&session, [event(1, api::EventKind::Ready), frame(2)]);
        supervise(
            || panic!("an actor defect"),
            &session.output,
            &session.closed,
            &session.document,
            &*session.wake,
        );
        assert_eq!(
            woken.load(Ordering::SeqCst),
            1,
            "an idle consumer learns of it"
        );
        assert!(!session.send(api::CommandKind::Copy), "the session stopped");
        let events = session.drain(8);
        assert!(is_runtime_error(&events), "{events:?}");
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
