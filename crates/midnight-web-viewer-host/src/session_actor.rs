//! The session actor forwards consumer commands to the browser, brokers its resource reads
//! and publishes its output. The actor, the reader of the browser's output and the
//! watchdog block on one monitor (`Signal`) until they have work, so an idle session wakes
//! none of them.
use super::*;
use crate::resource_jobs::ResourceJobs;
use std::{
    io::{self, Read, Write},
    sync::{Condvar, MutexGuard, PoisonError},
    time::{Duration, Instant},
};
#[cfg(test)]
#[path = "session_actor_tests.rs"]
mod tests;

/// Browser read results queued ahead of the actor. The reader waits for room instead of
/// reading on, so a browser that floods the host blocks in its own writes.
const BACKLOG: usize = 2;
/// Commands the actor forwards before it handles the next browser message.
const COMMANDS_PER_TURN: usize = 64;
const QUEUE_UNAVAILABLE: &str = "Browser command queue unavailable";
const WATCHDOG_UNAVAILABLE: &str = "Browser watchdog unavailable";

/// The browser process tree the watchdog ends.
pub(super) trait BrowserProcess: Send {
    /// Terminate the whole process tree. Termination may complete asynchronously.
    fn kill(&mut self) -> io::Result<()>;
    /// Wait until the browser and every process of its tree exited. `Ok(false)` reports
    /// a failed exit status.
    fn wait(&mut self) -> io::Result<bool>;
    /// Once `wait` returned for a browser that stopped before it reported Ready: why it
    /// could not start, when the platform can tell (Linux: shared libraries the system
    /// lacks). `None` otherwise.
    fn startup_failure(&mut self) -> Option<io::Error> {
        None
    }
}

impl BrowserProcess for midnight_worker_platform::browser::Process {
    fn kill(&mut self) -> io::Result<()> {
        midnight_worker_platform::browser::Process::kill(self)
    }
    fn wait(&mut self) -> io::Result<bool> {
        midnight_worker_platform::browser::Process::wait(self)
    }
    fn startup_failure(&mut self) -> Option<io::Error> {
        midnight_worker_platform::browser::Process::startup_failure(self)
    }
}

/// The browser's standard input and output, and its process tree.
pub(super) struct Transport {
    pub(super) input: Box<dyn Write + Send>,
    pub(super) output: Box<dyn Read + Send>,
    pub(super) process: Box<dyn BrowserProcess>,
}

/// How long the watchdog lets one write to the browser block, and the browser take to
/// report Ready, before it kills the process tree.
#[derive(Clone, Copy, Debug)]
pub(super) struct Limits {
    pub(super) busy: Duration,
    pub(super) startup: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            busy: Duration::from_secs(5),
            startup: Duration::from_secs(30),
        }
    }
}

/// The monitor of one session: what its actor, reader and watchdog wait for, under one
/// lock. Every predicate they wait on changes under this lock, or its writer locks and
/// unlocks it before notifying (see `interrupt`), so no wakeup is lost.
pub(super) struct Signal {
    inbox: Mutex<Inbox>,
    /// The actor: a command, browser output, the reader's end, a close or a stop.
    actor: Condvar,
    /// The reader: room in the browser backlog, a close or a stop.
    room: Condvar,
    /// The watchdog: a write it does not time yet, Ready, a close or a stop.
    watchdog: Condvar,
    /// Tests: one of the session's threads starts waiting.
    #[cfg(test)]
    observed: Condvar,
}

struct Inbox {
    commands: midnight_web_viewer_sdk::CommandQueue,
    /// Read results of the browser's output, at most `BACKLOG`.
    browser: VecDeque<io::Result<api::Message>>,
    /// A write to the browser is in progress; it started at `written`.
    writing: bool,
    /// When the latest write to the browser started.
    written: Option<Instant>,
    /// When the watchdog wakes by itself; `None` while it waits without a deadline.
    watch: Option<Instant>,
    /// The browser reported Ready, which ends the startup limit.
    ready: bool,
    /// The actor stopped: the reader and the watchdog end.
    stopping: bool,
    /// The reader ended. Once `browser` is empty, no further output comes.
    reader_done: bool,
    /// A resource completion or worker failure needs one actor turn.
    resources_ready: bool,
    #[cfg(test)]
    turns: Turns,
}

/// Tests: which of the session's threads wait, and how often the actor and the watchdog
/// woke.
#[cfg(test)]
#[derive(Debug, Default)]
struct Turns {
    actor_waiting: bool,
    actor_wakes: u64,
    reader_waiting: bool,
    watchdog_waiting: bool,
    watchdog_wakes: u64,
}

/// One turn of the actor, in this order: up to 64 commands, then one browser message.
struct Turn {
    commands: Vec<api::Command>,
    browser: Option<io::Result<api::Message>>,
}

impl Signal {
    pub(super) fn new(capacity: usize) -> Result<Self, String> {
        Ok(Self {
            inbox: Mutex::new(Inbox {
                commands: midnight_web_viewer_sdk::CommandQueue::new(capacity)
                    .map_err(|error| error.to_string())?,
                browser: VecDeque::with_capacity(BACKLOG),
                writing: false,
                written: None,
                watch: None,
                ready: false,
                stopping: false,
                reader_done: false,
                resources_ready: false,
                #[cfg(test)]
                turns: Turns::default(),
            }),
            actor: Condvar::new(),
            room: Condvar::new(),
            watchdog: Condvar::new(),
            #[cfg(test)]
            observed: Condvar::new(),
        })
    }

    /// The inbox, even after a panic poisoned its lock: each field stays valid between
    /// statements, and a stop must still reach every waiter.
    fn lock(&self) -> MutexGuard<'_, Inbox> {
        self.inbox.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Queue a consumer command and wake the actor. False when the queue is full.
    pub(super) fn push(&self, command: api::Command) -> bool {
        let pushed = self
            .inbox
            .lock()
            .is_ok_and(|mut inbox| inbox.commands.push(command).is_ok());
        if pushed {
            self.actor.notify_one();
        }
        pushed
    }

    pub(super) fn resources_ready(&self) {
        self.lock().resources_ready = true;
        self.actor.notify_one();
    }

    #[cfg(test)]
    pub(super) fn pop_command(&self) -> Option<api::Command> {
        self.lock().commands.pop()
    }

    /// Make a change of the session's `closed` flag visible to every waiter. A waiter that
    /// checked the flag before the change already waits once this lock is free, so the
    /// notification reaches it.
    pub(super) fn interrupt(&self) {
        drop(self.lock());
        self.actor.notify_all();
        self.room.notify_all();
        self.watchdog.notify_all();
    }

    /// Wait for the actor's next turn. `None` once the session is closed; an error once
    /// the reader ended without further output.
    fn take(&self, closed: &AtomicBool) -> Result<Option<Turn>, String> {
        let mut inbox = self.inbox.lock().map_err(|_| QUEUE_UNAVAILABLE)?;
        let turn = loop {
            if closed.load(Ordering::Acquire) {
                return Ok(None);
            }
            let commands: Vec<_> = (0..COMMANDS_PER_TURN)
                .map_while(|_| inbox.commands.pop())
                .collect();
            let browser = inbox.browser.pop_front();
            let resources_ready = std::mem::take(&mut inbox.resources_ready);
            if !commands.is_empty() || browser.is_some() || resources_ready {
                break Turn { commands, browser };
            }
            if inbox.reader_done {
                return Err("Chromium disconnected".into());
            }
            #[cfg(test)]
            {
                inbox.turns.actor_waiting = true;
                self.observed.notify_all();
            }
            inbox = self.actor.wait(inbox).map_err(|_| QUEUE_UNAVAILABLE)?;
            #[cfg(test)]
            {
                inbox.turns.actor_waiting = false;
                inbox.turns.actor_wakes += 1;
            }
        };
        drop(inbox);
        if turn.browser.is_some() {
            self.room.notify_one();
        }
        Ok(Some(turn))
    }

    /// A write to the browser starts. The watchdog learns of it only when it would not wake
    /// by this write's deadline anyway: it keeps timing the latest write for one limit
    /// after it started, so a burst of writes wakes it about once per limit, not per write.
    /// The notification is sent under the lock: sent after it, the decisions of writes
    /// that came before the watchdog set its deadline would each wake it again once it
    /// waits (CI saw four wakeups in a 0.2 ms burst).
    fn busy(&self, limit: Duration) -> Result<(), String> {
        let mut inbox = self.inbox.lock().map_err(|_| WATCHDOG_UNAVAILABLE)?;
        let now = Instant::now();
        inbox.writing = true;
        inbox.written = Some(now);
        let untimed = now
            .checked_add(limit)
            .is_some_and(|deadline| inbox.watch.is_none_or(|watch| watch > deadline));
        if untimed {
            self.watchdog.notify_one();
        }
        drop(inbox);
        Ok(())
    }

    /// The write in progress ended; the watchdog notices when it next wakes.
    fn idle(&self) -> Result<(), String> {
        self.inbox.lock().map_err(|_| WATCHDOG_UNAVAILABLE)?.writing = false;
        Ok(())
    }

    /// The browser reported Ready: the watchdog stops timing its startup.
    fn ready(&self) {
        let mut inbox = self.lock();
        let first = !std::mem::replace(&mut inbox.ready, true);
        drop(inbox);
        if first {
            self.watchdog.notify_one();
        }
    }

    /// Whether the browser reported Ready at least once.
    fn was_ready(&self) -> bool {
        self.lock().ready
    }
}

/// Stops the reader and the watchdog when dropped, also while the actor unwinds from a
/// panic. The watchdog then ends the browser's whole process tree, which releases blocked
/// pipe reads and writes, so the scope can join its threads.
struct Stop<'a>(&'a Signal);
impl Drop for Stop<'_> {
    fn drop(&mut self) {
        self.0.lock().stopping = true;
        self.0.actor.notify_all();
        self.0.room.notify_all();
        self.0.watchdog.notify_all();
    }
}

/// Marks the reader's end, also when it unwinds, so the actor never waits for output that
/// cannot come.
struct ReaderDone<'a>(&'a Signal);
impl Drop for ReaderDone<'_> {
    fn drop(&mut self) {
        self.0.lock().reader_done = true;
        self.0.actor.notify_one();
    }
}

struct Wire<'a> {
    input: Box<dyn Write + Send>,
    header: api::Header,
    signal: &'a Signal,
    /// How long one write may block (`Limits::busy`).
    limit: Duration,
}
impl Wire<'_> {
    fn send(&mut self, kind: api::CommandKind) -> Result<(), String> {
        self.header.sequence = self
            .header
            .sequence
            .checked_add(1)
            .ok_or("Browser sequence exhausted")?;
        self.signal.busy(self.limit)?;
        let result = api::write_message(
            &mut self.input,
            &api::Message::Command(api::Command {
                header: self.header,
                kind,
            }),
        )
        .map_err(|error| format!("Browser transport: {error}"));
        self.signal.idle()?;
        result
    }
}

/// Queue each read result of the browser's output for the actor, waiting for room instead
/// of reading on. It ends after a failed read, and once the actor stops or the session
/// closes.
fn read(signal: &Signal, closed: &AtomicBool, mut pipe: Box<dyn Read + Send>) {
    let _done = ReaderDone(signal);
    loop {
        let message = api::read_message(&mut pipe);
        let failed = message.is_err();
        let mut inbox = signal.lock();
        while inbox.browser.len() >= BACKLOG && !inbox.stopping && !closed.load(Ordering::Acquire) {
            #[cfg(test)]
            {
                inbox.turns.reader_waiting = true;
                signal.observed.notify_all();
            }
            inbox = signal
                .room
                .wait(inbox)
                .unwrap_or_else(PoisonError::into_inner);
            #[cfg(test)]
            {
                inbox.turns.reader_waiting = false;
            }
        }
        if inbox.stopping || closed.load(Ordering::Acquire) {
            return;
        }
        inbox.browser.push_back(message);
        drop(inbox);
        signal.actor.notify_one();
        if failed {
            return;
        }
    }
}

/// End the browser's process tree once the actor stops or the session closes, when one
/// write blocks longer than `limits.busy`, or when the browser is not Ready within
/// `limits.startup`. Once the browser is Ready and one limit passed since the latest
/// write started, it waits without a deadline.
fn watch(signal: &Signal, closed: &AtomicBool, limits: Limits, process: &mut dyn BrowserProcess) {
    let startup = Instant::now().checked_add(limits.startup);
    let mut inbox = signal.lock();
    loop {
        if inbox.stopping || closed.load(Ordering::Acquire) {
            break;
        }
        let now = Instant::now();
        let startup = startup.filter(|_| !inbox.ready);
        let write = inbox.written.and_then(|at| at.checked_add(limits.busy));
        if startup.is_some_and(|deadline| deadline <= now)
            || inbox.writing && write.is_some_and(|deadline| deadline <= now)
        {
            break;
        }
        // Timing the latest write past its end spares the writes that follow within the
        // limit a notification (see `Signal::busy`).
        let deadline = startup
            .into_iter()
            .chain(write.filter(|deadline| *deadline > now))
            .min();
        inbox.watch = deadline;
        let left = deadline.map(|deadline| deadline.duration_since(now));
        #[cfg(test)]
        {
            inbox.turns.watchdog_waiting = true;
            signal.observed.notify_all();
        }
        inbox = match left {
            None => signal
                .watchdog
                .wait(inbox)
                .unwrap_or_else(PoisonError::into_inner),
            Some(left) => {
                signal
                    .watchdog
                    .wait_timeout(inbox, left)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0
            }
        };
        #[cfg(test)]
        {
            inbox.turns.watchdog_waiting = false;
            inbox.turns.watchdog_wakes += 1;
        }
    }
    // Kill and wait with the lock released: waiting can take seconds, and `send` must
    // not block meanwhile.
    drop(inbox);
    let _ = process.kill();
    let _ = process.wait();
}

/// Why the actor stopped when no error stopped it.
pub(super) enum End {
    /// The session was closed, or the consumer sent Close.
    Requested,
    /// The browser ended the session with the protocol's Closed event.
    Browser,
}

/// How the actor treats a browser event whose header is valid.
#[derive(Debug, PartialEq)]
enum Admit {
    /// Output of the current document.
    Current,
    /// Output of a document the consumer replaced or is replacing.
    Stale,
    /// The protocol's Closed event, which ends the session.
    End,
}

/// Validate an event's header against the actor's `current` header (its session and
/// document) and the document's `last` event sequence, which an admitted event advances.
/// `requested` is the newest document the consumer asked for. The browser's Closed event
/// ends the session whatever document it names: a browser that stops before it reads the
/// Open of a replacement closes the document it last opened.
fn admit(
    event: &api::Event,
    current: api::Header,
    last: &mut u64,
    requested: u64,
) -> Result<Admit, String> {
    if event.header.session != current.session {
        return Err("Browser session mismatch".into());
    }
    let end = matches!(event.kind, api::EventKind::Closed);
    if event.header.document < current.document {
        return Ok(if end { Admit::End } else { Admit::Stale });
    }
    if event.header.document != current.document || event.header.sequence <= *last {
        return Err("Invalid browser generation/sequence".into());
    }
    *last = event.header.sequence;
    Ok(if end {
        Admit::End
    } else if event.header.document < requested {
        // UI permission changes revoke outstanding local-resource requests immediately,
        // even before the actor reaches the queued Open replacement.
        Admit::Stale
    } else {
        Admit::Current
    })
}

/// Start the provider's browser and run the session over its pipes.
pub(super) fn run(
    provider: Provider,
    owner: SourceOwner,
    viewport: api::Viewport,
    shared: &Shared,
) -> Result<End, String> {
    #[cfg(debug_assertions)]
    let validation_trace = {
        use std::sync::OnceLock;
        static ENABLED: OnceLock<bool> = OnceLock::new();
        static VALIDATION: AtomicU64 = AtomicU64::new(1);
        ENABLED
            .get_or_init(|| {
                std::env::var_os("MIDNIGHT_VIEWER_OPEN_TRACE").is_some_and(|value| value == "1")
            })
            .then(|| (VALIDATION.fetch_add(1, Ordering::Relaxed), Instant::now()))
    };
    let validation = provider.validate();
    #[cfg(debug_assertions)]
    let validation_trace = validation_trace.map(|(ordinal, started)| (ordinal, started.elapsed()));
    #[cfg(debug_assertions)]
    if let Some((ordinal, elapsed)) = validation_trace {
        let stderr = io::stderr();
        let _ = writeln!(
            stderr.lock(),
            "web_viewer_open validation={} phase=provider_validate elapsed_us={} success={}",
            ordinal,
            elapsed.as_micros(),
            u8::from(validation.is_ok()),
        );
    }
    validation.map_err(|error| error.to_string())?;
    let browser = midnight_worker_platform::browser::spawn_with_extensions(
        &provider.executable,
        launch_extensions(shared.links),
    )
    .map_err(|error| launch_failed(shared, &error))?;
    shared
        .browser
        .store(browser.process.id(), Ordering::Release);
    let transport = Transport {
        input: Box::new(browser.input),
        output: Box::new(browser.output),
        process: Box::new(browser.process),
    };
    // The rest of `browser`, its private profile, is dropped when this function returns:
    // after `run_with` waited for the whole process tree.
    run_with(transport, owner, viewport, shared, Limits::default())
}

/// The web-viewer API extensions this host accepts from a browser it launches: always the
/// physical-v1 shortcuts and cursor reports, and link routing only for a session that
/// routes links to the host.
fn launch_extensions(links: LinkRouting) -> midnight_worker_platform::browser::Extensions {
    midnight_worker_platform::browser::Extensions {
        host_shortcuts: true,
        links: links == LinkRouting::Host,
        cursor: true,
    }
}

/// The message of the terminal error of a browser that could not start. A refused sandbox
/// is recorded as that error's typed reason first: `supervise` publishes the error after
/// the actor returned, so a consumer that drains it finds the reason
/// (`Session::sandbox_refusal`).
pub(super) fn launch_failed(shared: &Shared, error: &io::Error) -> String {
    let (message, refusal) = startup_error(error);
    if let Ok(mut output) = shared.output.lock() {
        output.refusal = refusal;
    }
    message
}

/// A sandbox the system denies is reported with its own message, which names the one-time
/// fix, and its typed refusal for a consumer that offers that fix; missing system libraries
/// with their own message, which names them and how to install them; any other launch
/// failure keeps the generic prefix and has no refusal.
fn startup_error(error: &io::Error) -> (String, Option<crate::SandboxRefusal>) {
    use midnight_worker_platform::browser::{LibrariesMissing, SandboxUnavailable};
    let inner = error.get_ref();
    if let Some(unavailable) = inner.and_then(|inner| inner.downcast_ref::<SandboxUnavailable>()) {
        return (unavailable.to_string(), Some(unavailable.refusal().clone()));
    }
    if let Some(missing) = inner.and_then(|inner| inner.downcast_ref::<LibrariesMissing>()) {
        return (missing.to_string(), None);
    }
    (format!("Chromium startup: {error}"), None)
}

/// Run the session over `transport`: the actor on this thread, the reader of the browser's
/// output and the watchdog on scoped threads. Returns once all three ended, after the
/// watchdog killed and waited for the browser's process tree. A browser that failed before
/// it reported Ready may never have started: the platform's reason, when it has one,
/// replaces the stopped transport's message (see `BrowserProcess::startup_failure`).
pub(super) fn run_with(
    transport: Transport,
    owner: SourceOwner,
    mut viewport: api::Viewport,
    shared: &Shared,
    limits: Limits,
) -> Result<End, String> {
    let Transport {
        input,
        output,
        mut process,
    } = transport;
    let (signal, closed) = (&*shared.signal, &*shared.closed);
    let mut wire = Wire {
        input,
        header: api::Header {
            session: 1,
            document: 1,
            sequence: 0,
        },
        signal,
        limit: limits.busy,
    };
    let result = std::thread::scope(|scope| {
        let stop = Stop(signal);
        let watchdog = scope.spawn(|| watch(signal, closed, limits, &mut *process));
        let reader = scope.spawn(move || read(signal, closed, output));
        let resource_signal = shared.signal.clone();
        let mut store = ResourceJobs::with_wake(
            owner,
            shared.generation.clone(),
            Arc::new(move || resource_signal.resources_ready()),
        );
        let result = store
            .as_mut()
            .map_err(|error| error.clone())
            .and_then(|store| forward(&mut wire, store, &mut viewport, shared));
        shared.generation.close();
        drop(stop);
        let _ = watchdog.join();
        let _ = reader.join();
        match store {
            Ok(store) => store.finish().and(result),
            Err(_) => result,
        }
    });
    // The watchdog waited for the process tree, so its exit status is known here.
    match result {
        Err(message) if !signal.was_ready() && !closed.load(Ordering::Acquire) => Err(process
            .startup_failure()
            .map_or(message, |error| launch_failed(shared, &error))),
        result => result,
    }
}

/// The actor: open the document, then forward commands and handle the browser's output
/// until the session ends.
fn forward(
    wire: &mut Wire<'_>,
    store: &mut ResourceJobs,
    viewport: &mut api::Viewport,
    shared: &Shared,
) -> Result<End, String> {
    let main = store.reset(1, shared.policy.network)?.main;
    wire.send(api::CommandKind::Open {
        source: main,
        viewport: *viewport,
        policy: shared.policy,
    })?;
    let mut event_sequence = 0;
    while let Some(turn) = shared.signal.take(&shared.closed)? {
        for command in turn.commands {
            validate_frontend_command(shared.ceiling, &command.kind)?;
            match command.kind {
                api::CommandKind::Policy(next) => {
                    let document = wire
                        .header
                        .document
                        .checked_add(1)
                        .ok_or("Browser document IDs exhausted")?;
                    shared.generation.ensure_at_least(document)?;
                    let reset = store.reset(document, next.network)?;
                    for reply in reset.denied {
                        send_resource_reply(
                            &shared.generation,
                            &shared.output,
                            &shared.wake,
                            wire.header.document,
                            reply,
                            |reply| wire.send(api::CommandKind::ResourceReply(reply)),
                        )?;
                    }
                    wire.header.document = document;
                    wire.header.sequence = 0;
                    event_sequence = 0;
                    shared
                        .output
                        .lock()
                        .map_err(|_| "Browser output unavailable")?
                        .frame = None;
                    wire.send(api::CommandKind::Open {
                        source: reset.main,
                        viewport: *viewport,
                        policy: next,
                    })?;
                }
                api::CommandKind::Resize(next) => {
                    if next.revision <= viewport.revision {
                        continue;
                    }
                    *viewport = next;
                    wire.send(api::CommandKind::Resize(next))?;
                }
                api::CommandKind::Close => return Ok(End::Requested),
                other => wire.send(other)?,
            }
        }
        for reply in store.drain()? {
            send_resource_reply(
                &shared.generation,
                &shared.output,
                &shared.wake,
                wire.header.document,
                reply,
                |reply| wire.send(api::CommandKind::ResourceReply(reply)),
            )?;
        }
        if shared.generation.is_closed() && !shared.closed.load(Ordering::Acquire) {
            return Err("HTML resource generation was revoked; reopen the document.".into());
        }
        let Some(incoming) = turn.browser else {
            continue;
        };
        let incoming = incoming.map_err(|error| format!("Chromium transport stopped: {error}"))?;
        let api::Message::Event(event) = incoming else {
            return Err("Unexpected browser command".into());
        };
        let requested = shared.document.load(Ordering::Acquire);
        match admit(&event, wire.header, &mut event_sequence, requested)? {
            Admit::Current => {}
            Admit::Stale => continue,
            Admit::End => return Ok(End::Browser),
        }
        let reply = match &event.kind {
            api::EventKind::ResourceResolve { request, base, url } => {
                store.resolve_with_header(event.header, *request, *base, url)
            }
            api::EventKind::ResourceRead {
                request,
                resource,
                offset,
                length,
            } => store.read_with_header(event.header, *request, *resource, *offset, *length),
            api::EventKind::ResourceRelease { resource } => store.release(*resource),
            _ => None,
        };
        if let Some(reply) = reply {
            send_resource_reply(
                &shared.generation,
                &shared.output,
                &shared.wake,
                wire.header.document,
                reply,
                |reply| wire.send(api::CommandKind::ResourceReply(reply)),
            )?;
        }
        if matches!(
            event.kind,
            api::EventKind::ResourceResolve { .. }
                | api::EventKind::ResourceRead { .. }
                | api::EventKind::ResourceRelease { .. }
        ) {
            continue;
        }
        match &event.kind {
            api::EventKind::Frame(frame) => {
                if frame.acknowledged_input > wire.header.sequence {
                    return Err("Browser acknowledged unsent input".into());
                }
                wire.send(api::CommandKind::FrameAck {
                    frame_sequence: event.header.sequence,
                })?;
                if frame.viewport_revision != viewport.revision {
                    continue;
                }
                if frame.width != viewport.width || frame.height != viewport.height {
                    return Err("Browser frame dimensions do not match viewport".into());
                }
            }
            api::EventKind::Ready => shared.signal.ready(),
            // A provider sends link events only under the launch opt-in (`LinkRouting`).
            api::EventKind::LinkHover { .. } | api::EventKind::LinkActivated { .. }
                if shared.links == LinkRouting::Browser =>
            {
                return Err(
                    "Browser sent a link event without the link-routing launch opt-in".into(),
                );
            }
            _ => {}
        }
        shared
            .output
            .lock()
            .map_err(|_| "Browser output unavailable")?
            .put(event)?;
        (shared.wake)();
    }
    Ok(End::Requested)
}

#[test]
fn a_browser_closed_event_ends_the_session_whatever_document_it_names() {
    let current = api::Header {
        session: 1,
        document: 2,
        sequence: 7,
    };
    let event = |document, sequence, kind| api::Event {
        header: api::Header {
            session: 1,
            document,
            sequence,
        },
        kind,
    };
    let mut last = 3;
    // A replaced document's output is stale, but not its Closed event: a browser that stops
    // before it reads the replacement's Open closes the document it last opened.
    let stale = event(1, 9, api::EventKind::Ready);
    assert_eq!(admit(&stale, current, &mut last, 2), Ok(Admit::Stale));
    let replaced = event(1, 10, api::EventKind::Closed);
    assert_eq!(admit(&replaced, current, &mut last, 2), Ok(Admit::End));
    // The current document's output, its end included, follows its sequence.
    let ready = event(2, 4, api::EventKind::Ready);
    assert_eq!(admit(&ready, current, &mut last, 2), Ok(Admit::Current));
    assert!(admit(&event(2, 4, api::EventKind::Closed), current, &mut last, 2).is_err());
    // The output of a document the consumer is replacing is stale, but its end is not.
    let revoked = event(2, 5, api::EventKind::Ready);
    assert_eq!(admit(&revoked, current, &mut last, 3), Ok(Admit::Stale));
    let replacing = event(2, 6, api::EventKind::Closed);
    assert_eq!(admit(&replacing, current, &mut last, 3), Ok(Admit::End));
    // Neither a document the host has not opened nor another session ends this one.
    assert!(admit(&event(3, 7, api::EventKind::Closed), current, &mut last, 3).is_err());
    let mut foreign = event(2, 8, api::EventKind::Closed);
    foreign.header.session = 2;
    assert!(admit(&foreign, current, &mut last, 3).is_err());
}

#[test]
fn physical_key_launch_contract_matches_the_public_api() {
    assert_eq!(
        midnight_worker_platform::browser::HOST_SHORTCUTS_ENV,
        api::HOST_SHORTCUTS_ENV
    );
    assert_eq!(
        midnight_worker_platform::browser::HOST_SHORTCUTS_PHYSICAL_V1,
        api::HOST_SHORTCUTS_PHYSICAL_V1
    );
}

#[test]
fn only_a_session_that_routes_links_to_the_host_launches_with_the_link_opt_in() {
    use midnight_worker_platform::browser::{Extensions, LINKS_ENV, LINKS_HOST_V1};
    assert_eq!(
        (LINKS_ENV, LINKS_HOST_V1),
        (api::LINKS_ENV, api::LINKS_HOST_V1)
    );
    let launched = |links| {
        let extensions = launch_extensions(links);
        (extensions, extensions.environment().collect::<Vec<_>>())
    };
    assert_eq!(LinkRouting::default(), LinkRouting::Browser);
    assert_eq!(
        launched(LinkRouting::Browser),
        (
            Extensions {
                host_shortcuts: true,
                links: false,
                cursor: true
            },
            vec![
                (api::HOST_SHORTCUTS_ENV, api::HOST_SHORTCUTS_PHYSICAL_V1),
                (api::CURSOR_ENV, api::CURSOR_V1)
            ]
        )
    );
    assert_eq!(
        launched(LinkRouting::Host),
        (
            Extensions {
                host_shortcuts: true,
                links: true,
                cursor: true
            },
            vec![
                (api::HOST_SHORTCUTS_ENV, api::HOST_SHORTCUTS_PHYSICAL_V1),
                (api::LINKS_ENV, api::LINKS_HOST_V1),
                (api::CURSOR_ENV, api::CURSOR_V1)
            ]
        )
    );
    assert_eq!(
        (
            midnight_worker_platform::browser::CURSOR_ENV,
            midnight_worker_platform::browser::CURSOR_V1
        ),
        (api::CURSOR_ENV, api::CURSOR_V1)
    );
}

/// Only a successful matching Denied write can produce the one source-change
/// correlation pair. The original renderer header is cause metadata, not an ack.
pub(super) fn send_resource_reply(
    generation: &Generation,
    output: &Arc<Mutex<Output>>,
    wake: &Arc<dyn Fn() -> bool + Send + Sync>,
    document: u64,
    reply: api::ResourceReply,
    send: impl FnOnce(api::ResourceReply) -> Result<(), String>,
) -> Result<(), String> {
    let denied = match &reply {
        api::ResourceReply::Denied { request, .. } => Some(*request),
        _ => None,
    };
    send(reply)?;
    if let Some(request) = denied
        && let Some(origin) = generation.take_revocation(document, request)
    {
        output
            .lock()
            .map_err(|_| "Browser output unavailable")?
            .put_source_changed(origin)?;
        wake();
    }
    Ok(())
}

/// The actor checks the immutable ceiling even for commands bypassing Session admission.
/// Any policy is allowed under the user's choice: scripts may run offline (web viewer API
/// 0.4), where the provider keeps them from every network.
pub(super) fn validated_policy(
    ceiling: PolicyCeiling,
    next: api::Policy,
) -> Result<api::Policy, String> {
    if ceiling == PolicyCeiling::OfflineOnly {
        return Err("This document has a fixed offline policy".into());
    }
    Ok(next)
}

pub(super) fn validate_frontend_command(
    ceiling: PolicyCeiling,
    command: &api::CommandKind,
) -> Result<(), String> {
    match command {
        api::CommandKind::Policy(policy) => validated_policy(ceiling, *policy).map(|_| ()),
        api::CommandKind::Open { .. }
        | api::CommandKind::ResourceReply(_)
        | api::CommandKind::FrameAck { .. } => Err("Reserved browser command".into()),
        _ => Ok(()),
    }
}
