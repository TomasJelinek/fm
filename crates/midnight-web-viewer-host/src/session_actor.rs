use super::*;
use crate::resource_jobs::ResourceJobs;
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

pub(super) fn terminal_exit(requested_close: bool, reason: &str) -> Result<(), String> {
    if requested_close {
        Ok(())
    } else {
        Err(reason.into())
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run(
    provider: Provider,
    owner: SourceOwner,
    ceiling: PolicyCeiling,
    mut viewport: api::Viewport,
    commands: Arc<Mutex<midnight_web_viewer_sdk::CommandQueue>>,
    output: &Arc<Mutex<Output>>,
    closed: &Arc<AtomicBool>,
    requested_document: &Arc<AtomicU64>,
    generation: &Arc<Generation>,
    wake: &Arc<dyn Fn() -> bool + Send + Sync>,
) -> Result<(), String> {
    provider.validate().map_err(|error| error.to_string())?;
    let browser =
        midnight_worker_platform::browser::spawn_with_host_shortcuts(&provider.executable)
            .map_err(|error| format!("Chromium startup: {error}"))?;
    let mut store = ResourceJobs::new(owner, generation.clone())?;
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
    let result = std::thread::scope(|scope| {
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
            let main = store.reset(1, false)?.main;
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
                    validate_frontend_command(ceiling, &command.kind)?;
                    match command.kind {
                        api::CommandKind::Policy(next) => {
                            let document = wire
                                .header
                                .document
                                .checked_add(1)
                                .ok_or("Browser document IDs exhausted")?;
                            generation.ensure_at_least(document)?;
                            let reset = store.reset(document, next.network)?;
                            // Every accepted request receives a terminal denial under its original
                            // document before the renderer is told to replace that document.
                            for reply in reset.denied {
                                send_resource_reply(
                                    generation,
                                    output,
                                    wake,
                                    wire.header.document,
                                    reply,
                                    |reply| wire.send(api::CommandKind::ResourceReply(reply)),
                                )?;
                            }
                            wire.header.document = document;
                            wire.header.sequence = 0;
                            event_sequence = 0;
                            output
                                .lock()
                                .map_err(|_| "Browser output unavailable")?
                                .frame = None;
                            wire.send(api::CommandKind::Open {
                                source: reset.main,
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
                for reply in store.drain()? {
                    send_resource_reply(
                        generation,
                        output,
                        wake,
                        wire.header.document,
                        reply,
                        |reply| wire.send(api::CommandKind::ResourceReply(reply)),
                    )?;
                }
                let incoming = match incoming.recv_timeout(Duration::from_millis(5)) {
                    Ok(value) => match value {
                        Ok(message) => message,
                        Err(error) => {
                            return terminal_exit(
                                closed.load(Ordering::Acquire),
                                &format!("Chromium transport stopped: {error}"),
                            );
                        }
                    },
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(_) => {
                        return terminal_exit(
                            closed.load(Ordering::Acquire),
                            "Chromium disconnected",
                        );
                    }
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
                match &event.kind {
                    api::EventKind::ResourceResolve { request, base, url } => {
                        if let Some(reply) =
                            store.resolve_with_header(event.header, *request, *base, url)
                        {
                            send_resource_reply(
                                generation,
                                output,
                                wake,
                                wire.header.document,
                                reply,
                                |reply| wire.send(api::CommandKind::ResourceReply(reply)),
                            )?;
                        }
                        continue;
                    }
                    api::EventKind::ResourceRead {
                        request,
                        resource,
                        offset,
                        length,
                    } => {
                        if let Some(reply) = store.read_with_header(
                            event.header,
                            *request,
                            *resource,
                            *offset,
                            *length,
                        ) {
                            send_resource_reply(
                                generation,
                                output,
                                wake,
                                wire.header.document,
                                reply,
                                |reply| wire.send(api::CommandKind::ResourceReply(reply)),
                            )?;
                        }
                        continue;
                    }
                    api::EventKind::ResourceRelease { resource } => {
                        if let Some(reply) = store.release(*resource) {
                            send_resource_reply(
                                generation,
                                output,
                                wake,
                                wire.header.document,
                                reply,
                                |reply| wire.send(api::CommandKind::ResourceReply(reply)),
                            )?;
                        }
                        continue;
                    }
                    _ => {}
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
                    api::EventKind::Closed => {
                        return terminal_exit(
                            closed.load(Ordering::Acquire),
                            "Chromium closed unexpectedly",
                        );
                    }
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
        generation.close();
        // Closing the broker's entire process tree releases blocked pipe reads/writes.
        stopping.store(true, Ordering::Release);
        drop(incoming);
        let _ = supervisor.join();
        let _ = reader.join();
        result
    });
    store.finish().and(result)
}

/// The actor checks the immutable ceiling even for commands bypassing Session admission.
pub(super) fn validated_policy(
    ceiling: PolicyCeiling,
    next: api::Policy,
) -> Result<api::Policy, String> {
    if ceiling == PolicyCeiling::OfflineOnly {
        return Err("This document has a fixed offline policy".into());
    }
    if next.scripts && !next.network {
        return Err("Scripts require online mode".into());
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
