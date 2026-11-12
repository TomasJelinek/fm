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
) -> Result<(), String> {
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
                        api::CommandKind::Close => return Ok(()),
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
                if event.header.session != wire.header.session {
                    return Err("Browser session mismatch".into());
                }
                if event.header.document < wire.header.document {
                    continue;
                }
                if event.header.document != wire.header.document
                    || event.header.sequence <= event_sequence
                {
                    return Err("Invalid browser generation/sequence".into());
                }
                event_sequence = event.header.sequence;
                // UI permission changes revoke outstanding local-resource requests immediately,
                // even before the actor reaches the queued Open replacement.
                if event.header.document < requested_document.load(Ordering::Acquire) {
                    continue;
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
                    api::EventKind::Closed => return Ok(()),
                    _ => {}
                }
                output
                    .lock()
                    .map_err(|_| "Browser output unavailable")?
                    .put(event)?;
                wake();
            }
            Ok(())
        })();
        // Closing the broker's entire process tree releases blocked pipe reads/writes.
        stopping.store(true, Ordering::Release);
        drop(incoming);
        let _ = supervisor.join();
        let _ = reader.join();
        result
    })
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
