use crate::{
    CapturedFile, EventKind, HostEvent,
    api::{abi, codec, messages, viewer, wire},
    callbacks::{Bridge, Service, ServiceKind},
    content::Reader,
    loaded::{self, Loaded},
    session::{self, Command, Shared, lock},
};
use std::{
    sync::{
        Arc,
        atomic::Ordering,
        mpsc::{self, Receiver, SyncSender},
    },
    time::Duration,
};

struct Reply {
    id: u64,
    reply: messages::ReadReply,
    bytes: Vec<u8>,
}

pub(crate) fn run(
    loaded: Arc<Loaded>,
    reader: Reader,
    file: CapturedFile,
    shared: Arc<Shared>,
    commands: Receiver<Command>,
) {
    let (bridge, receivers) = Bridge::new(shared.clone());
    let Ok((_registration, host)) = bridge.register() else {
        shared.fatal(1, "Host callback registration failed".into());
        return;
    };
    let (replies, reply_rx) = mpsc::sync_channel(32);
    let mut workers = Vec::new();
    for (name, receive) in [("viewer-cpu", receivers.cpu), ("viewer-io", receivers.io)] {
        let bridge = bridge.clone();
        let loaded = loaded.clone();
        match std::thread::Builder::new()
            .name(name.into())
            .spawn(move || session::worker(bridge, loaded, receive))
        {
            Ok(worker) => workers.push(worker),
            Err(error) => {
                shared.fatal(1, error.to_string());
                break;
            }
        }
    }
    if workers.len() == 2 {
        let bridge = bridge.clone();
        match std::thread::Builder::new()
            .name("viewer-read".into())
            .spawn(move || services(bridge, reader, file, receivers.service, replies))
        {
            Ok(worker) => workers.push(worker),
            Err(error) => shared.fatal(1, error.to_string()),
        }
    }
    if workers.len() != 3 {
        bridge.stop.store(true, Ordering::Release);
        for worker in workers {
            let _ = worker.join();
        }
        return;
    }
    let mut instance = 0;
    // SAFETY: host callbacks remain registered until joined workers and destroyed instance.
    let status = unsafe { loaded.api.create.unwrap()(&host, loaded::slice(&[]), &mut instance) };
    if status != abi::OK || instance == 0 {
        shared.fatal(1, format!("Plugin create failed ({status})"));
        bridge.stop.store(true, Ordering::Release);
        for worker in workers {
            let _ = worker.join();
        }
        return;
    }
    let mut shutting_down = false;
    let mut pending_reply: Option<crate::replies::PendingReply> = None;
    loop {
        let closing = shared.closing.load(Ordering::Acquire);
        if closing && !shutting_down {
            shared.discard_output(); // Large model cleanup remains on backend before quota release.
            // SAFETY: dispatcher serializes every instance entry and pins the loaded library.
            unsafe {
                loaded.api.begin_shutdown.unwrap()(instance);
            }
            shutting_down = true;
        }
        let cancelled: Vec<_> = lock(&shared.state)
            .requests
            .iter()
            .filter(|(_, cancelled)| **cancelled)
            .map(|(id, _)| *id)
            .collect();
        for id in cancelled {
            unsafe {
                loaded.api.cancel.unwrap()(instance, id);
            }
        }
        if shared.room() {
            for _ in 0..8 {
                let Ok(command) = commands.try_recv() else {
                    break;
                };
                if shared.cancelled(command.id) {
                    shared.push(
                        HostEvent {
                            request_id: command.id,
                            generation: shared.generation,
                            kind: EventKind::Cancelled,
                        },
                        0,
                    );
                    continue;
                }
                let request = abi::Request {
                    id: command.id,
                    session: 1,
                    generation: shared.generation,
                    kind: command.kind,
                    flags: 0,
                    payload: loaded::slice(&command.payload),
                    content: loaded::empty(),
                };
                // SAFETY: request data are borrowed only until submit returns.
                let status = unsafe { loaded.api.submit.unwrap()(instance, &request) };
                if status != abi::OK {
                    shared.failure(command.id, format!("Plugin rejected request ({status})"));
                }
            }
        }
        for _ in 0..32 {
            if let Some(pending) = pending_reply.take() {
                // SAFETY: pending owns the initial content lease until accepted or rejected.
                if !pending.deliver(&bridge, |request| unsafe {
                    loaded.api.submit.unwrap()(instance, request)
                }) {
                    pending_reply = Some(pending);
                    break;
                }
            }
            let Ok(mut reply) = reply_rx.try_recv() else {
                break;
            };
            let buffer = match bridge.allocate(std::mem::take(&mut reply.bytes)) {
                Ok(buffer) => buffer,
                Err(status) => {
                    reply.reply.status = status;
                    reply.reply.bytes = 0;
                    reply.reply.eof = false;
                    loaded::empty()
                }
            };
            if let Ok(payload) = codec::encode(&reply.reply) {
                pending_reply = Some(crate::replies::PendingReply {
                    id: reply.id,
                    payload,
                    buffer,
                });
            } else {
                bridge.release(buffer.id);
                bridge.services.fetch_sub(1, Ordering::Release);
                shared.fatal(1, "Host read service produced an invalid reply".into());
            }
        }
        for _ in 0..32 {
            if !shared.room() {
                break;
            }
            // SAFETY: table of integer/pointer values has valid zeroed representation.
            let mut event: abi::Event = unsafe { std::mem::zeroed() };
            let mut written = 0;
            // SAFETY: exactly one aligned writable Event and count are exclusively borrowed.
            let status =
                unsafe { loaded.api.drain.unwrap()(instance, 1, &mut event, &mut written) };
            if written > 1 {
                shared.fatal(1, "Plugin drain exceeded supplied event capacity".into());
                break;
            }
            if written == 1 {
                receive_event(&loaded, instance, &shared, event);
            }
            if status != abi::OK {
                shared.fatal(1, format!("Plugin drain failed ({status})"));
                break;
            }
            if written == 0 {
                break;
            }
        }
        if shutting_down && bridge.empty() {
            // SDK v1 reports quiescence using NOT_QUIESCENT until destroy can succeed.
            // No forced unload: uncooperative native code keeps its bounded session permit.
            let status = unsafe { loaded.api.destroy.unwrap()(instance) };
            if status == abi::OK {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    bridge.stop.store(true, Ordering::Release);
    for worker in workers {
        let _ = worker.join();
    }
}

fn services(
    bridge: Arc<Bridge>,
    reader: Reader,
    file: CapturedFile,
    receive: Receiver<Service>,
    send: SyncSender<Reply>,
) {
    let mut reader = Some(reader);
    while !bridge.stop.load(Ordering::Acquire) {
        let Ok(service) = receive.recv_timeout(Duration::from_millis(10)) else {
            continue;
        };
        let mut reply = messages::ReadReply {
            revision: file.revision(),
            offset: 0,
            bytes: 0,
            eof: false,
            status: abi::OK,
        };
        let mut bytes = Vec::new();
        match service.kind {
            ServiceKind::Read(read) => {
                reply.revision = read.revision;
                reply.offset = read.offset;
                if bridge.shared.closing.load(Ordering::Acquire) {
                    reply.status = abi::SHUTTING_DOWN;
                } else if read.reader != 1 || read.revision != file.revision() {
                    reply.status = abi::UNKNOWN_HANDLE;
                } else if let Some(reader) = &mut reader {
                    match reader.read_at(read.offset, read.length) {
                        Ok(read_bytes) => {
                            bytes = read_bytes;
                            reply.bytes = bytes.len() as u32;
                            reply.eof =
                                read.offset.saturating_add(bytes.len() as u64) >= file.length();
                        }
                        Err(_) => reply.status = abi::INVALID,
                    }
                } else {
                    reply.status = abi::UNKNOWN_HANDLE;
                }
            }
            ServiceKind::Close(close) => {
                if close.reader == 1 {
                    reader = None;
                } else {
                    reply.status = abi::UNKNOWN_HANDLE;
                }
            }
        }
        // A full reply queue blocks only this service worker. Dispatch always drains replies,
        // including during shutdown, and no scheduler worker permit is held while waiting.
        if send
            .send(Reply {
                id: service.id,
                reply,
                bytes,
            })
            .is_err()
        {
            break;
        }
    }
}

fn receive_event(loaded: &Loaded, instance: u64, shared: &Shared, event: abi::Event) {
    let result = decode_event(event);
    // SAFETY: release exactly the two plugin-owned buffer IDs on backend, even on invalid/stale data.
    unsafe {
        if event.payload.id != 0 {
            loaded.api.release_buffer.unwrap()(instance, event.payload.id);
        }
        if event.content.id != 0 && event.content.id != event.payload.id {
            loaded.api.release_buffer.unwrap()(instance, event.content.id);
        }
    }
    if event.session != 1 || event.generation != shared.generation {
        return;
    }
    if !lock(&shared.state).requests.contains_key(&event.request_id) {
        return;
    }
    match result {
        Ok(Some((mut kind, bytes))) => {
            if !valid_revision(&kind, shared) {
                shared.fatal(
                    event.request_id,
                    "Plugin output does not match captured content".into(),
                );
                return;
            }
            if shared.cancelled(event.request_id)
                && !matches!(
                    kind,
                    EventKind::Completed | EventKind::Cancelled | EventKind::Failed(_)
                )
            {
                return;
            }
            if shared.cancelled(event.request_id) && matches!(kind, EventKind::Completed) {
                kind = EventKind::Cancelled;
            }
            shared.push(
                HostEvent {
                    request_id: event.request_id,
                    generation: event.generation,
                    kind,
                },
                bytes,
            );
        }
        Ok(None) => {}
        Err(_) => {
            shared.fatal(event.request_id, "Malformed plugin output".into());
        }
    }
}

fn decode_event(event: abi::Event) -> Result<Option<(EventKind, usize)>, crate::Error> {
    event
        .payload
        .validate(wire::MAX_MESSAGE_BYTES as u32)
        .map_err(|_| crate::Error("Invalid payload buffer".into()))?;
    event
        .content
        .validate(viewer::MAX_PAGE_BYTES as u32)
        .map_err(|_| crate::Error("Invalid content buffer".into()))?;
    if event.flags != 0 {
        return Err(crate::Error("Unknown required event flags".into()));
    }
    // SAFETY: trusted plugin transfers immutable mapped bytes until release_buffer.
    let payload = unsafe { loaded::bytes(event.payload.bytes, wire::MAX_MESSAGE_BYTES as u32) }?;
    let content = unsafe { loaded::bytes(event.content.bytes, viewer::MAX_PAGE_BYTES as u32) }?;
    let invalid = |_| crate::Error("Invalid event model".into());
    let kind = match event.kind {
        abi::EVENT_OPENED => EventKind::Opened(codec::decode(payload).map_err(invalid)?),
        abi::EVENT_TEXT_PAGE => EventKind::TextPage(codec::decode(payload).map_err(invalid)?),
        abi::EVENT_DOCUMENT_PAGE => {
            EventKind::DocumentPage(codec::decode(payload).map_err(invalid)?)
        }
        abi::EVENT_BINARY_PAGE => {
            let page: viewer::BinaryPage = codec::decode(payload).map_err(invalid)?;
            if page.source.length != content.len() as u64 {
                return Err(crate::Error("Binary page/content mismatch".into()));
            }
            EventKind::BinaryPage {
                page,
                bytes: content.to_vec(),
            }
        }
        abi::EVENT_SEARCH_MATCH => EventKind::SearchMatch(codec::decode(payload).map_err(invalid)?),
        abi::EVENT_PROGRESS => EventKind::Progress(codec::decode(payload).map_err(invalid)?),
        abi::EVENT_FAILED => EventKind::Failed(codec::decode(payload).map_err(invalid)?),
        abi::EVENT_COMPLETED if payload.is_empty() => EventKind::Completed,
        abi::EVENT_CANCELLED if payload.is_empty() => EventKind::Cancelled,
        abi::EVENT_QUIESCENT => return Ok(None),
        abi::EVENT_COMPLETED | abi::EVENT_CANCELLED => {
            return Err(crate::Error("Invalid terminal payload".into()));
        }
        _ => return Ok(None),
    };
    if !matches!(kind, EventKind::BinaryPage { .. }) && !content.is_empty() {
        return Err(crate::Error("Unexpected event content".into()));
    }
    // Charge actual decoded vector/string capacity, not compact wire representation.
    let retained = kind.retained_bytes();
    Ok(Some((kind, retained)))
}

fn valid_revision(kind: &EventKind, shared: &Shared) -> bool {
    let within = |source: &viewer::SourceRange| source.end().is_ok_and(|end| end <= shared.length);
    match kind {
        EventKind::Opened(opened) => {
            opened.mode == shared.mode && opened.length.is_none_or(|length| length == shared.length)
        }
        EventKind::TextPage(page) => {
            matches!(shared.mode, viewer::MODE_TEXT | viewer::MODE_SOURCE)
                && page.revision == shared.revision
                && within(&page.source)
        }
        EventKind::BinaryPage { page, .. } => {
            shared.mode == viewer::MODE_BINARY
                && page.revision == shared.revision
                && within(&page.source)
        }
        EventKind::DocumentPage(page) => {
            shared.mode == viewer::MODE_DOCUMENT
                && page.revision == shared.revision
                && within(&page.source)
        }
        EventKind::SearchMatch(found) => within(&found.source),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn document_budget_accounts_for_decoded_block_and_anchor_allocations() {
        use crate::api::document::{BLOCK_RULE, Block, DocumentPage};
        let text = viewer::TextPage {
            revision: 9,
            source: viewer::SourceRange {
                offset: 0,
                length: 0,
            },
            text: String::new(),
            spans: Vec::new(),
            anchors: vec![viewer::Anchor {
                text_offset: 0,
                source_offset: 0,
            }],
            has_before: false,
            has_after: false,
            partial_line: false,
        };
        let page = DocumentPage {
            revision: 9,
            source: viewer::SourceRange {
                offset: 0,
                length: 0,
            },
            blocks: vec![
                Block {
                    kind: BLOCK_RULE,
                    depth: 0,
                    heading_level: 0,
                    text,
                    links: Vec::new()
                };
                512
            ],
            has_before: false,
            has_after: false,
        };
        let payload = codec::encode(&page).unwrap();
        let event = abi::Event {
            request_id: 1,
            session: 1,
            generation: 7,
            kind: abi::EVENT_DOCUMENT_PAGE,
            flags: 0,
            payload: abi::Buffer {
                id: 1,
                bytes: loaded::slice(&payload),
            },
            content: loaded::empty(),
        };
        let (_, charged) = decode_event(event).unwrap().unwrap();
        assert!(
            charged >= 512 * (size_of::<Block>() + size_of::<viewer::Anchor>()),
            "decoded allocation was charged as compact wire bytes"
        );
    }
    #[test]
    fn pages_must_match_captured_revision_mode_and_source_bounds() {
        let mut shared = crate::session::tests::shared();
        Arc::get_mut(&mut shared).unwrap().mode = viewer::MODE_BINARY;
        let mut page = viewer::BinaryPage {
            revision: 9,
            source: viewer::SourceRange {
                offset: 60,
                length: 4,
            },
            has_before: true,
            has_after: false,
        };
        assert!(valid_revision(
            &EventKind::BinaryPage {
                page: page.clone(),
                bytes: vec![0; 4]
            },
            &shared
        ));
        page.revision = 8;
        assert!(!valid_revision(
            &EventKind::BinaryPage {
                page: page.clone(),
                bytes: vec![0; 4]
            },
            &shared
        ));
        page.revision = 9;
        page.source.offset = 61;
        assert!(!valid_revision(
            &EventKind::BinaryPage {
                page: page.clone(),
                bytes: vec![0; 4]
            },
            &shared
        ));
        page.source.offset = 60;
        Arc::get_mut(&mut shared).unwrap().mode = viewer::MODE_TEXT;
        assert!(!valid_revision(
            &EventKind::BinaryPage {
                page,
                bytes: vec![0; 4]
            },
            &shared
        ));
    }
}
