use super::*;
use crate::resources::Store;
use std::{
    fs::File,
    time::{Duration, Instant},
};

struct Wire {
    input: File,
    header: api::Header,
    busy: Arc<Mutex<Option<Instant>>>,
}
impl Wire {
    fn send(&mut self, kind: api::CommandKind) -> Result<(), String> {
        self.header.sequence = self
            .header
            .sequence
            .checked_add(1)
            .ok_or("Browser sequence exhausted")?;
        *self
            .busy
            .lock()
            .map_err(|_| "Browser watchdog unavailable")? = Some(Instant::now());
        let result = api::write_message(
            &mut self.input,
            &api::Message::Command(api::Command {
                header: self.header,
                kind,
            }),
        )
        .map_err(|error| format!("Browser transport: {error}"));
        *self
            .busy
            .lock()
            .map_err(|_| "Browser watchdog unavailable")? = None;
        result
    }
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

/// Stops the supervisor when dropped, also while the session loop unwinds from a panic.
/// The supervisor then closes the broker's entire process tree, which releases blocked
/// pipe reads/writes, so the scope can join its threads.
struct Stop<'a>(&'a AtomicBool);
impl Drop for Stop<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run(
    provider: Provider,
    source: CapturedFile,
    resources: Option<Arc<dyn Resources>>,
    mut viewport: api::Viewport,
    commands: Arc<Mutex<midnight_web_viewer_sdk::CommandQueue>>,
    output: &Arc<Mutex<Output>>,
    closed: &Arc<AtomicBool>,
    requested_document: &Arc<AtomicU64>,
    wake: &Arc<dyn Fn() -> bool + Send + Sync>,
) -> Result<End, String> {
    provider.validate().map_err(|error| error.to_string())?;
    let browser =
        midnight_worker_platform::browser::spawn_with_host_shortcuts(&provider.executable)
            .map_err(|error| format!("Chromium startup: {error}"))?;
    let mut store = Store::new(source, resources);
    let mut wire = Wire {
        input: browser.input,
        header: api::Header {
            session: 1,
            document: 1,
            sequence: 0,
        },
        busy: Arc::new(Mutex::new(None)),
    };
    let stopping = Arc::new(AtomicBool::new(false));
    let ready = Arc::new(AtomicBool::new(false));
    let mut process = browser.process;
    std::thread::scope(|scope| {
        let stop = Stop(&stopping);
        let stopping = &stopping;
        let busy = wire.busy.clone();
        let ready = &ready;
        let supervisor = scope.spawn(move || {
            let started = Instant::now();
            while !stopping.load(Ordering::Acquire) && !closed.load(Ordering::Acquire) {
                let expired = busy.lock().map_or(true, |at| {
                    at.is_some_and(|at| at.elapsed() > Duration::from_secs(5))
                });
                if expired
                    || !ready.load(Ordering::Acquire) && started.elapsed() > Duration::from_secs(30)
                {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let _ = process.kill();
            let _ = process.wait();
        });
        let (sender, incoming) = mpsc::sync_channel(2);
        let reader = scope.spawn(move || {
            let mut pipe = browser.output;
            loop {
                let message = api::read_message(&mut pipe);
                let failed = message.is_err();
                if sender.send(message).is_err() || failed {
                    break;
                }
            }
        });
        let result = (|| {
            let main = store.reset(false)?;
            wire.send(api::CommandKind::Open {
                source: main,
                viewport,
                policy: api::Policy::default(),
            })?;
            let mut event_sequence = 0;
            while !closed.load(Ordering::Acquire) {
                let batch: Vec<_> = {
                    let mut commands = commands
                        .lock()
                        .map_err(|_| "Browser command queue unavailable")?;
                    (0..64).map_while(|_| commands.pop()).collect()
                };
                for command in batch {
                    match command.kind {
                        api::CommandKind::Policy(next) => {
                            if next.scripts && !next.network {
                                return Err("Scripts require online mode".into());
                            }
                            wire.header.document = wire
                                .header
                                .document
                                .checked_add(1)
                                .ok_or("Browser document IDs exhausted")?;
                            wire.header.sequence = 0;
                            event_sequence = 0;
                            output
                                .lock()
                                .map_err(|_| "Browser output unavailable")?
                                .frame = None;
                            let source = store.reset(next.network)?;
                            wire.send(api::CommandKind::Open {
                                source,
                                viewport,
                                policy: next,
                            })?;
                        }
                        api::CommandKind::Resize(next) => {
                            if next.revision <= viewport.revision {
                                continue;
                            }
                            viewport = next;
                            wire.send(api::CommandKind::Resize(next))?;
                        }
                        api::CommandKind::Close => return Ok(End::Requested),
                        other => wire.send(other)?,
                    }
                }
                let incoming = match incoming.recv_timeout(Duration::from_millis(5)) {
                    Ok(value) => {
                        value.map_err(|error| format!("Chromium transport stopped: {error}"))?
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(_) => return Err("Chromium disconnected".into()),
                };
                let api::Message::Event(event) = incoming else {
                    return Err("Unexpected browser command".into());
                };
                let requested = requested_document.load(Ordering::Acquire);
                match admit(&event, wire.header, &mut event_sequence, requested)? {
                    Admit::Current => {}
                    Admit::Stale => continue,
                    Admit::End => return Ok(End::Browser),
                }
                let reply = match &event.kind {
                    api::EventKind::ResourceResolve { request, base, url } => {
                        Some(store.resolve(*request, *base, url))
                    }
                    api::EventKind::ResourceRead {
                        request,
                        resource,
                        offset,
                        length,
                    } => Some(store.read(*request, *resource, *offset, *length)),
                    api::EventKind::ResourceRelease { resource } => Some(store.release(*resource)),
                    _ => None,
                };
                if let Some(reply) = reply {
                    wire.send(api::CommandKind::ResourceReply(reply))?;
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
                    api::EventKind::Ready => ready.store(true, Ordering::Release),
                    _ => {}
                }
                output
                    .lock()
                    .map_err(|_| "Browser output unavailable")?
                    .put(event)?;
                wake();
            }
            Ok(End::Requested)
        })();
        drop(stop);
        drop(incoming);
        let _ = supervisor.join();
        let _ = reader.join();
        result
    })
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
