//! The production session thread (`Session::with_transport`, which runs `run_with` behind
//! `supervise` and the exit guard) over an in-memory transport: a scripted browser on
//! bounded pipes whose process handle closes both pipes when killed, as ending a real
//! process tree closes its pipe ends. Every wait for an expected event is bounded by a
//! deadline that only elapses when a test fails.
use super::*;
use std::{
    sync::{OnceLock, mpsc},
    thread,
};

/// Bounds every wait for an expected event.
const WAIT: Duration = Duration::from_secs(10);
/// Bytes one pipe buffers before a write blocks.
const PIPE_CAPACITY: usize = 64 * 1024;
const VIEWPORT: api::Viewport = api::Viewport {
    revision: 1,
    width: 4,
    height: 3,
    scale: 1.0,
};

/// A test's state, also after a failed assertion poisoned its lock, so cleanup in `Drop`
/// cannot panic again.
fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Wait until `done` holds for the state behind `mutex`, rechecked on each notification.
fn await_state<'a, T>(
    mutex: &'a Mutex<T>,
    condvar: &Condvar,
    what: &str,
    mut done: impl FnMut(&T) -> bool,
) -> MutexGuard<'a, T> {
    let (state, waited) = condvar
        .wait_timeout_while(locked(mutex), WAIT, |state| !done(state))
        .unwrap_or_else(PoisonError::into_inner);
    if waited.timed_out() {
        // Unlock first: a failure must not poison the state the cleanup still uses.
        drop(state);
        panic!("timed out waiting for {what}");
    }
    state
}

#[derive(Default)]
struct PipeState {
    bytes: VecDeque<u8>,
    /// The writing end is closed: reads end once the buffer is drained.
    written: bool,
    /// The reading end is closed: writes fail.
    unread: bool,
}

/// One direction of a bounded in-memory pipe.
#[derive(Default)]
struct Pipe {
    state: Mutex<PipeState>,
    changed: Condvar,
}

impl Pipe {
    fn ends() -> (Arc<Pipe>, PipeReader, PipeWriter) {
        let pipe = Arc::new(Pipe::default());
        (pipe.clone(), PipeReader(pipe.clone()), PipeWriter(pipe))
    }
    fn state(&self) -> MutexGuard<'_, PipeState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
    /// Close both ends, as ending the process that holds them does.
    fn close(&self) {
        let mut state = self.state();
        state.written = true;
        state.unread = true;
        drop(state);
        self.changed.notify_all();
    }
}

struct PipeReader(Arc<Pipe>);
impl Read for PipeReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let mut state = self.0.state();
        while state.bytes.is_empty() && !state.written && !buffer.is_empty() {
            state = self.0.changed.wait(state).unwrap();
        }
        let count = buffer.len().min(state.bytes.len());
        for (slot, byte) in buffer.iter_mut().zip(state.bytes.drain(..count)) {
            *slot = byte;
        }
        drop(state);
        self.0.changed.notify_all();
        Ok(count)
    }
}
impl Drop for PipeReader {
    fn drop(&mut self) {
        self.0.state().unread = true;
        self.0.changed.notify_all();
    }
}

struct PipeWriter(Arc<Pipe>);
impl Write for PipeWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let mut state = self.0.state();
        loop {
            if state.unread {
                return Err(io::ErrorKind::BrokenPipe.into());
            }
            let room = PIPE_CAPACITY - state.bytes.len();
            if room > 0 || buffer.is_empty() {
                let count = room.min(buffer.len());
                state.bytes.extend(&buffer[..count]);
                drop(state);
                self.0.changed.notify_all();
                return Ok(count);
            }
            state = self.0.changed.wait(state).unwrap();
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Drop for PipeWriter {
    fn drop(&mut self) {
        self.0.state().written = true;
        self.0.changed.notify_all();
    }
}

/// The scripted browser's event writer, numbered like a provider: each Open starts a
/// document whose events count from 1.
struct Events {
    pipe: Option<PipeWriter>,
    document: u64,
    sequence: u64,
}
impl Events {
    fn emit(&mut self, kind: api::EventKind) -> io::Result<()> {
        self.sequence += 1;
        let pipe = self.pipe.as_mut().ok_or(io::ErrorKind::BrokenPipe)?;
        let header = api::Header {
            session: 1,
            document: self.document,
            sequence: self.sequence,
        };
        api::write_message(pipe, &api::Message::Event(api::Event { header, kind }))
    }
}

/// What the scripted browser does by itself.
#[derive(Clone, Copy, PartialEq)]
enum Script {
    /// Read commands and answer each Open with Ready and a frame of its viewport.
    Ready,
    /// Read commands and never report Ready.
    Silent,
    /// Read nothing, like a browser that hangs.
    Stalled,
}

/// The scripted browser's process: killing it closes both pipes; waiting joins its thread.
struct Process {
    pipes: [Arc<Pipe>; 2],
    thread: Option<thread::JoinHandle<()>>,
    /// The commands pipe of a stalled browser, open and unread.
    _stalled: Option<PipeReader>,
    kills: Arc<AtomicU64>,
}
impl BrowserProcess for Process {
    fn kill(&mut self) -> io::Result<()> {
        self.kills.fetch_add(1, Ordering::SeqCst);
        for pipe in &self.pipes {
            pipe.close();
        }
        Ok(())
    }
    fn wait(&mut self) -> io::Result<bool> {
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| io::Error::other("the scripted browser panicked"))?;
        }
        Ok(true)
    }
}

/// The far side of a session's transport.
struct Browser {
    events: Arc<Mutex<Events>>,
    /// The commands the browser read, in order.
    received: mpsc::Receiver<api::Command>,
    /// How often the session killed the browser.
    kills: Arc<AtomicU64>,
}
impl Browser {
    fn emit(&self, kind: api::EventKind) {
        locked(&self.events).emit(kind).unwrap();
    }
    fn receive(&self) -> api::CommandKind {
        self.received.recv_timeout(WAIT).expect("a command").kind
    }
    fn kills(&self) -> u64 {
        self.kills.load(Ordering::SeqCst)
    }
}

fn painted(viewport: &api::Viewport) -> api::EventKind {
    api::EventKind::Frame(api::Frame {
        viewport_revision: viewport.revision,
        acknowledged_input: 0,
        width: viewport.width,
        height: viewport.height,
        rgba: vec![0; viewport.width as usize * viewport.height as usize * 4],
    })
}

fn browser(script: Script) -> (Transport, Browser) {
    let (commands, from_host, to_browser) = Pipe::ends();
    let (output, from_browser, to_host) = Pipe::ends();
    let events = Arc::new(Mutex::new(Events {
        pipe: Some(to_host),
        document: 1,
        sequence: 0,
    }));
    let (sender, received) = mpsc::channel();
    let (thread, stalled) = if script == Script::Stalled {
        (None, Some(from_host))
    } else {
        let events = events.clone();
        let thread = thread::spawn(move || {
            let mut input = from_host;
            while let Ok(api::Message::Command(command)) = api::read_message(&mut input) {
                if let api::CommandKind::Open { viewport, .. } = &command.kind {
                    let mut events = locked(&events);
                    events.document = command.header.document;
                    events.sequence = 0;
                    if script == Script::Ready {
                        let _ = events.emit(api::EventKind::Ready);
                        let _ = events.emit(painted(viewport));
                    }
                }
                let _ = sender.send(command);
            }
        });
        (Some(thread), None)
    };
    let kills = Arc::new(AtomicU64::new(0));
    let transport = Transport {
        input: Box::new(to_browser),
        output: Box::new(from_browser),
        process: Box::new(Process {
            pipes: [commands, output],
            thread,
            _stalled: stalled,
            kills: kills.clone(),
        }),
    };
    (
        transport,
        Browser {
            events,
            received,
            kills,
        },
    )
}

/// The main document of the sessions; it names no relative resources.
#[derive(Debug)]
struct Document;
impl midnight_plugin_host::ContentSource for Document {
    fn length(&self) -> u64 {
        5
    }
    fn display_name(&self) -> String {
        "index.html".into()
    }
    fn version(&self) -> Vec<u8> {
        vec![1]
    }
    fn validate(&self) -> Result<(), midnight_plugin_host::Error> {
        Ok(())
    }
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, midnight_plugin_host::Error> {
        let end = offset.saturating_add(u64::from(length)).min(5) as usize;
        Ok(b"hello"[(offset as usize).min(end)..end].to_vec())
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), midnight_plugin_host::Error> {
        drop(self);
        Ok(())
    }
}

fn document() -> CapturedFile {
    CapturedFile::from_source(Arc::new(Document)).unwrap()
}

/// A resource scope whose capture reports that it started, waits for the test, then
/// panics (a defect in the actor's thread) or denies the resource.
#[derive(Debug)]
struct Capture {
    entered: Mutex<mpsc::Sender<()>>,
    release: Mutex<mpsc::Receiver<()>>,
    panics: bool,
}
impl Resources for Capture {
    fn capture(&self, _: &[String]) -> Result<CapturedFile, String> {
        let _ = locked(&self.entered).send(());
        let _ = locked(&self.release).recv_timeout(WAIT);
        assert!(!self.panics, "a resource capture defect");
        Err("denied".into())
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), String> {
        drop(self);
        Ok(())
    }
}

/// A capture scope, its signal that a capture started and the release it waits for.
fn capture(panics: bool) -> (Arc<dyn Resources>, mpsc::Receiver<()>, mpsc::Sender<()>) {
    let (entered, started) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let capture = Capture {
        entered: Mutex::new(entered),
        release: Mutex::new(released),
        panics,
    };
    (Arc::new(capture), started, release)
}

/// The consumer: counts the session's wakes, records whether the session had published
/// its exit at each one, and can hold the session's thread inside a wake.
#[derive(Default)]
struct Consumer {
    state: Mutex<Woken>,
    changed: Condvar,
    exited: OnceLock<Arc<AtomicBool>>,
}
#[derive(Default)]
struct Woken {
    count: u64,
    /// Whether the session had published its exit at each wake, once known.
    exited: Vec<Option<bool>>,
    /// Hold the session's thread when it next wakes the consumer.
    hold: bool,
    held: bool,
}
impl Consumer {
    fn wake(&self) -> bool {
        let exited = self
            .exited
            .get()
            .map(|exited| exited.load(Ordering::Acquire));
        let mut state = locked(&self.state);
        state.count += 1;
        state.exited.push(exited);
        if state.hold && thread::current().name() == Some("viewer-html") {
            state.held = true;
            self.changed.notify_all();
            state = self
                .changed
                .wait_timeout_while(state, WAIT, |state| state.hold)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
            state.held = false;
        }
        drop(state);
        self.changed.notify_all();
        true
    }
    fn hold(&self) {
        locked(&self.state).hold = true;
    }
    fn release(&self) {
        locked(&self.state).hold = false;
        self.changed.notify_all();
    }
    /// Wait until `done`, rechecked on each wake.
    fn until(&self, what: &str, done: impl FnMut(&Woken) -> bool) {
        drop(await_state(&self.state, &self.changed, what, done));
    }
    fn count(&self) -> u64 {
        locked(&self.state).count
    }
}

struct Harness {
    session: Session,
    browser: Browser,
    consumer: Arc<Consumer>,
}
impl Drop for Harness {
    // A failed assertion must not leave the session's thread held while dropping joins it.
    fn drop(&mut self) {
        self.consumer.release();
    }
}

fn open_with(
    transport: Transport,
    browser: Browser,
    source: CapturedFile,
    resources: Option<Arc<dyn Resources>>,
    limits: Limits,
) -> Harness {
    let consumer = Arc::new(Consumer::default());
    let wake = {
        let consumer = consumer.clone();
        Arc::new(move || consumer.wake())
    };
    let session =
        Session::with_transport(transport, source, resources, VIEWPORT, limits, wake).unwrap();
    consumer.exited.set(session.exited.clone()).unwrap();
    Harness {
        session,
        browser,
        consumer,
    }
}

fn open(script: Script, resources: Option<Arc<dyn Resources>>, limits: Limits) -> Harness {
    let (transport, browser) = browser(script);
    open_with(transport, browser, document(), resources, limits)
}

impl Harness {
    /// The browser answered Open with Ready and a frame, which the host acknowledged and
    /// published.
    fn shown(&self) {
        assert!(matches!(
            self.browser.receive(),
            api::CommandKind::Open { .. }
        ));
        assert!(matches!(
            self.browser.receive(),
            api::CommandKind::FrameAck { frame_sequence: 2 }
        ));
        self.consumer
            .until("Ready and the first frame", |woken| woken.count >= 2);
    }
    /// The session's thread ended and published its exit.
    fn finished(&self) {
        self.consumer
            .until("the session's exit", |_| self.session.finished());
    }
    /// Wait until the actor waits for work.
    fn actor_idle(&self) {
        let signal = &self.session.signal;
        drop(await_state(
            &signal.inbox,
            &signal.observed,
            "an idle actor",
            |inbox| inbox.turns.actor_waiting,
        ));
    }
    /// Wait until the actor waits for work and the watchdog waits without a deadline
    /// (one busy limit after the latest write); return how often each woke so far.
    fn quiescent(&self) -> (u64, u64) {
        let signal = &self.session.signal;
        let inbox = await_state(
            &signal.inbox,
            &signal.observed,
            "an idle session",
            |inbox| {
                inbox.turns.actor_waiting && inbox.turns.watchdog_waiting && inbox.watch.is_none()
            },
        );
        (inbox.turns.actor_wakes, inbox.turns.watchdog_wakes)
    }
    fn kinds(&self) -> Vec<api::EventKind> {
        let events = self.session.drain(64).into_iter();
        events.map(|event| event.kind).collect()
    }
}

fn shown() -> Vec<api::EventKind> {
    vec![api::EventKind::Ready, painted(&VIEWPORT)]
}

fn runtime_error(kinds: &[api::EventKind]) -> &str {
    match kinds {
        [
            api::EventKind::Error {
                code: api::ErrorCode::Runtime,
                message,
            },
        ] => message,
        other => panic!("expected one runtime error, got {other:?}"),
    }
}

fn state() -> api::EventKind {
    api::EventKind::State(api::State {
        title: String::new(),
        url: String::new(),
        loading: false,
        editable: false,
        zoom: 1.0,
    })
}

fn resolve() -> api::EventKind {
    api::EventKind::ResourceResolve {
        request: 1,
        base: 1,
        url: "style.css".into(),
    }
}

#[test]
fn a_close_command_ends_the_session_as_requested() {
    let harness = open(Script::Ready, None, Limits::default());
    harness.shown();
    assert!(harness.session.send(api::CommandKind::Close));
    harness.finished();
    assert_eq!(harness.browser.kills(), 1, "the watchdog ended the browser");
    assert_eq!(
        harness.kinds(),
        shown(),
        "a requested end publishes nothing"
    );
    assert!(!harness.session.send(api::CommandKind::Copy));
}

#[test]
fn closing_an_idle_session_wakes_its_actor_and_ends_it_as_requested() {
    let harness = open(Script::Ready, None, Limits::default());
    harness.shown();
    harness.actor_idle();
    let woken = harness.consumer.count();
    harness.session.close();
    // Without the close's notification the actor would wait forever.
    harness.finished();
    assert_eq!(harness.browser.kills(), 1);
    assert_eq!(
        harness.kinds(),
        shown(),
        "a requested end publishes nothing"
    );
    assert_eq!(harness.consumer.count(), woken + 1, "only the exit wakes");
}

#[test]
fn a_browser_closed_event_ends_the_session_as_browser() {
    let harness = open(Script::Ready, None, Limits::default());
    harness.shown();
    harness.browser.emit(api::EventKind::Closed);
    harness.finished();
    assert_eq!(harness.browser.kills(), 1);
    assert_eq!(
        harness.kinds(),
        [shown(), vec![api::EventKind::Closed]].concat()
    );
}

#[test]
fn a_transport_eof_fails_the_session() {
    let harness = open(Script::Ready, None, Limits::default());
    harness.shown();
    locked(&harness.browser.events).pipe = None;
    harness.finished();
    let kinds = harness.kinds();
    let message = runtime_error(&kinds);
    assert!(
        message.starts_with("Chromium transport stopped"),
        "{message}"
    );
}

#[test]
fn a_reader_that_ends_without_output_fails_the_session() {
    struct Defective;
    impl Read for Defective {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            panic!("a reader defect")
        }
    }
    let (mut transport, browser) = browser(Script::Silent);
    transport.output = Box::new(Defective);
    let harness = open_with(transport, browser, document(), None, Limits::default());
    harness.finished();
    assert_eq!(harness.browser.kills(), 1);
    assert_eq!(runtime_error(&harness.kinds()), "Chromium disconnected");
}

#[test]
fn a_resource_worker_panic_wakes_the_actor_and_ends_the_watchdog() {
    let (resources, started, release) = capture(true);
    release.send(()).unwrap();
    let harness = open(Script::Ready, Some(resources), Limits::default());
    harness.shown();
    harness.browser.emit(resolve());
    started.recv_timeout(WAIT).unwrap();
    harness.finished();
    assert_eq!(harness.browser.kills(), 1, "the watchdog ended the browser");
    assert_eq!(
        runtime_error(&harness.kinds()),
        "HTML resource worker panicked; cleanup is unconfirmed"
    );
}

/// A resource worker is inside capture. Hold the actor inside a presentation wake,
/// so the reader queues two messages and waits for room with a third.
fn flooded(panics: bool) -> (Harness, mpsc::Sender<()>) {
    let (resources, started, release) = capture(panics);
    let harness = open(Script::Ready, Some(resources), Limits::default());
    harness.shown();
    harness.browser.emit(resolve());
    started.recv_timeout(WAIT).unwrap();
    harness.consumer.hold();
    harness.browser.emit(state());
    harness.consumer.until("the actor held", |woken| woken.held);
    for _ in 0..4 {
        harness.browser.emit(state());
    }
    let signal = &harness.session.signal;
    drop(await_state(
        &signal.inbox,
        &signal.observed,
        "a reader waiting for room",
        |inbox| inbox.turns.reader_waiting,
    ));
    (harness, release)
}

#[test]
fn closing_a_flooded_session_ends_it() {
    let (harness, release) = flooded(false);
    harness.session.close();
    release.send(()).unwrap();
    harness.consumer.release();
    harness.finished();
    assert_eq!(harness.browser.kills(), 1);
    assert_eq!(
        harness.kinds(),
        [shown(), vec![state()]].concat(),
        "a requested end publishes nothing"
    );
}

#[test]
fn a_resource_worker_that_panics_while_the_browser_floods_still_ends_the_session() {
    let (harness, release) = flooded(true);
    // Only the stop guard can wake the reader, which waits for room.
    release.send(()).unwrap();
    harness.consumer.release();
    harness.finished();
    assert_eq!(harness.browser.kills(), 1);
    assert_eq!(
        runtime_error(&harness.kinds()),
        "HTML resource worker panicked; cleanup is unconfirmed"
    );
}

#[test]
fn an_idle_session_makes_no_actor_or_watchdog_turns() {
    // A short busy limit ends the watchdog's timing of the first frame's acknowledgement.
    let limits = Limits {
        busy: Duration::from_millis(100),
        ..Limits::default()
    };
    let harness = open(Script::Ready, None, limits);
    harness.shown();
    let before = harness.quiescent();
    // An observation window, not synchronization: nothing may happen during it. Condvar
    // waits may return spuriously, so two wakeups each are tolerated; a 5 ms poll would
    // wake the actor about 40 times.
    thread::sleep(Duration::from_millis(200));
    let after = harness.quiescent();
    assert!(
        after.0 - before.0 <= 2 && after.1 - before.1 <= 2,
        "idle wakeups (actor, watchdog): before {before:?}, after {after:?}"
    );
    assert_eq!(harness.consumer.count(), 2, "no output, no wake");
}

#[test]
fn a_command_wakes_an_idle_actor() {
    let harness = open(Script::Ready, None, Limits::default());
    harness.shown();
    harness.actor_idle();
    assert!(harness.session.send(api::CommandKind::Copy));
    assert!(matches!(harness.browser.receive(), api::CommandKind::Copy));
}

#[test]
fn a_resource_completion_wakes_an_idle_actor_without_browser_output() {
    let (resources, started, release) = capture(false);
    let harness = open(Script::Ready, Some(resources), Limits::default());
    harness.shown();
    harness.browser.emit(resolve());
    started.recv_timeout(WAIT).unwrap();
    harness.actor_idle();
    release.send(()).unwrap();
    assert!(matches!(
        harness.browser.receive(),
        api::CommandKind::ResourceReply(api::ResourceReply::Denied { request: 1, .. })
    ));
    assert_eq!(
        harness.consumer.count(),
        2,
        "resource replies do not repaint the UI"
    );
}

#[test]
fn a_burst_of_writes_wakes_the_watchdog_about_once_per_limit() {
    let limits = Limits {
        busy: Duration::from_millis(300),
        ..Limits::default()
    };
    let harness = open(Script::Ready, None, limits);
    harness.shown();
    let (_, before) = harness.quiescent();
    let burst = Instant::now();
    for _ in 0..20 {
        assert!(harness.session.send(api::CommandKind::Copy));
    }
    for _ in 0..20 {
        assert!(matches!(harness.browser.receive(), api::CommandKind::Copy));
    }
    let after = harness
        .session
        .signal
        .inbox
        .lock()
        .unwrap()
        .turns
        .watchdog_wakes;
    let elapsed = burst.elapsed();
    // The first write wakes the watchdog, which then times it; the writes that follow
    // within the limit need no notification. Its own timer wakes it once per limit the
    // burst spans, which a stalled runner stretches (CI once saw 3 wakes). One spurious
    // wakeup is tolerated; a notification per write would be about 20.
    let spanned = u64::try_from(elapsed.as_nanos() / limits.busy.as_nanos()).unwrap_or(u64::MAX);
    assert!(
        after - before <= 2 + spanned,
        "watchdog wakeups: {before} -> {after} in {elapsed:?}"
    );
}

#[test]
fn input_overflow_stops_the_actor() {
    let harness = open(Script::Ready, None, Limits::default());
    harness.shown();
    // Hold the actor inside the wake for this event, so it cannot take commands.
    harness.consumer.hold();
    harness.browser.emit(state());
    harness.consumer.until("the actor held", |woken| woken.held);
    for _ in 0..QUEUED_COMMANDS {
        assert!(harness.session.send(api::CommandKind::Copy));
    }
    assert!(
        !harness.session.send(api::CommandKind::Copy),
        "the queue is full"
    );
    harness.consumer.release();
    harness.finished();
    assert_eq!(harness.browser.kills(), 1);
    let kinds = harness.kinds();
    assert!(runtime_error(&kinds).contains("overflowed"), "{kinds:?}");
}

#[test]
fn a_blocked_write_is_killed_after_the_busy_limit() {
    // A ResourceReply of the largest read, from a file, outgrows the pipe of a browser
    // that stopped reading.
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("index.html");
    std::fs::write(&path, vec![b'x'; api::MAX_READ_BYTES]).unwrap();
    let source = CapturedFile::capture(&path).unwrap();
    let limits = Limits {
        busy: Duration::from_millis(200),
        ..Limits::default()
    };
    let (transport, browser) = browser(Script::Stalled);
    let harness = open_with(transport, browser, source, None, limits);
    harness.browser.emit(api::EventKind::Ready);
    harness.consumer.until("Ready", |woken| woken.count >= 1);
    let started = Instant::now();
    harness.browser.emit(api::EventKind::ResourceRead {
        request: 1,
        resource: 1,
        offset: 0,
        length: api::MAX_READ_BYTES as u32,
    });
    harness.finished();
    assert!(started.elapsed() >= limits.busy, "{:?}", started.elapsed());
    assert_eq!(harness.browser.kills(), 1);
    let kinds = harness.kinds();
    let message = runtime_error(&kinds);
    assert!(message.starts_with("Browser transport"), "{message}");
}

#[test]
fn a_browser_that_never_becomes_ready_is_killed_after_the_startup_limit() {
    let limits = Limits {
        startup: Duration::from_millis(100),
        ..Limits::default()
    };
    let started = Instant::now();
    let harness = open(Script::Silent, None, limits);
    assert!(matches!(
        harness.browser.receive(),
        api::CommandKind::Open { .. }
    ));
    harness.finished();
    assert!(
        started.elapsed() >= limits.startup,
        "{:?}",
        started.elapsed()
    );
    assert_eq!(harness.browser.kills(), 1);
    let kinds = harness.kinds();
    let message = runtime_error(&kinds);
    assert!(
        message.starts_with("Chromium transport stopped"),
        "{message}"
    );
}

#[test]
fn the_session_thread_marks_itself_finished_before_its_last_wake() {
    let harness = open(Script::Ready, None, Limits::default());
    harness.shown();
    harness.session.close();
    harness.consumer.until("the exit wake", |woken| {
        woken.exited.last() == Some(&Some(true))
    });
    let Harness {
        session, consumer, ..
    } = &harness;
    assert!(session.finished());
    let exited = locked(&consumer.state).exited.clone();
    assert_eq!(
        exited
            .iter()
            .filter(|exited| **exited == Some(true))
            .count(),
        1,
        "only the last wake follows the exit: {exited:?}"
    );
}

#[test]
fn a_denied_sandbox_is_reported_with_its_own_fix_and_other_launch_failures_keep_the_prefix() {
    let fix = "The HTML viewer needs permission for Chromium's sandbox on this system. \
               Run once: sudo /opt/me/install-html-viewer-apparmor.sh";
    let refusal = crate::SandboxRefusal::UserNamespaces {
        executable: "/opt/me/viewers/midnight-viewer-html/midnight-viewer-html".into(),
    };
    let denied = io::Error::new(
        io::ErrorKind::PermissionDenied,
        midnight_worker_platform::browser::SandboxUnavailable::new(fix, refusal.clone()),
    );
    assert_eq!(startup_error(&denied), (fix.to_owned(), Some(refusal)));
    let missing = io::Error::new(io::ErrorKind::NotFound, "No such file or directory");
    assert_eq!(
        startup_error(&missing),
        (
            "Chromium startup: No such file or directory".to_owned(),
            None
        )
    );
}
