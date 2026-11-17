use crate::{
    CapturedFile, EventKind, HostEvent,
    api::{abi, codec, messages, viewer, wire},
    callbacks::{Bridge, Service, ServiceKind},
    content::Reader,
    loaded::{self, Loaded},
    replies::PendingReply,
    session::{self, Command, OPEN_REQUEST_ID, Shared, lock},
};
use std::{
    sync::{
        Arc,
        atomic::Ordering,
        mpsc::{self, Receiver, SyncSender},
    },
    thread::JoinHandle,
    time::Duration,
};

/// Every producer notifies the dispatcher. This fallback only lets a raw-ABI plugin
/// that publishes output without `notify_ready` make progress.
const IDLE_POLL: Duration = Duration::from_millis(250);
/// A QUEUE_FULL service reply and NOT_QUIESCENT shutdown have no completion signal.
const RETRY_POLL: Duration = Duration::from_millis(10);
const COMMAND_BATCH: usize = 8;
const REPLY_BATCH: usize = 32;
const EVENT_BATCH: usize = 32;

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
    let dispatcher = std::thread::current();
    shared.dispatcher.bind(Arc::new(move || {
        dispatcher.unpark();
        true
    }));
    let (bridge, receivers) = Bridge::new(shared.clone());
    let Ok((_registration, host)) = bridge.register() else {
        shared.fatal(OPEN_REQUEST_ID, "Host callback registration failed".into());
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
                shared.fatal(OPEN_REQUEST_ID, error.to_string());
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
            Err(error) => shared.fatal(OPEN_REQUEST_ID, error.to_string()),
        }
    }
    if workers.len() != 3 {
        stop(&bridge, reply_rx, workers);
        return;
    }
    let api = &loaded.api;
    let mut instance = 0;
    // SAFETY: host callbacks remain registered until joined workers and destroyed instance.
    let status = unsafe { api.create.unwrap()(&host, loaded::slice(&[]), &mut instance) };
    if status != abi::OK || instance == 0 {
        shared.fatal(OPEN_REQUEST_ID, format!("Plugin create failed ({status})"));
        stop(&bridge, reply_rx, workers);
        return;
    }
    let mut shutting_down = false;
    let mut pending_reply: Option<PendingReply> = None;
    loop {
        shared.dispatcher.reset();
        let closing = shared.closing.load(Ordering::Acquire);
        if closing && !shutting_down {
            shared.discard_output(); // Large model cleanup remains on backend before quota release.
            // SAFETY: dispatcher serializes every instance entry and pins the loaded library.
            unsafe {
                api.begin_shutdown.unwrap()(instance);
            }
            shutting_down = true;
        }
        if !shutting_down {
            // begin_shutdown itself cancels every outstanding request.
            deliver_cancels(api, instance, &shared);
        }
        let mut busy = submit_commands(api, instance, &shared, &commands);
        busy |= deliver_replies(api, instance, &bridge, &reply_rx, &mut pending_reply);
        busy |= drain_events(api, instance, &shared);
        if shutting_down && bridge.empty() {
            // SDK v1 reports quiescence using NOT_QUIESCENT until destroy can succeed.
            // No forced unload: uncooperative native code keeps its bounded session permit.
            let status = unsafe { api.destroy.unwrap()(instance) };
            if status == abi::OK {
                break;
            }
        }
        if !busy {
            // A notification that arrived after `reset` has already unparked this thread.
            std::thread::park_timeout(if shutting_down || pending_reply.is_some() {
                RETRY_POLL
            } else {
                IDLE_POLL
            });
        }
    }
    stop(&bridge, reply_rx, workers);
}

/// Close worker admission and join. Dropping the reply receiver releases a service
/// worker blocked on a full reply queue; jobs and services already accepted drain.
fn stop(bridge: &Bridge, replies: Receiver<Reply>, workers: Vec<JoinHandle<()>>) {
    bridge.close();
    drop(replies);
    for worker in workers {
        let _ = worker.join();
    }
}

/// Deliver native cancellation once per submitted request. A request whose command is
/// still queued is converted to Cancelled by `submit_commands` and never reaches the plugin.
fn deliver_cancels(api: &abi::PluginApi, instance: u64, shared: &Shared) {
    let cancelled: Vec<_> = lock(&shared.state)
        .requests
        .iter_mut()
        .filter(|(_, request)| request.cancelled && request.submitted && !request.cancel_delivered)
        .map(|(id, request)| {
            request.cancel_delivered = true;
            *id
        })
        .collect();
    for id in cancelled {
        // SAFETY: dispatcher serializes every instance entry and pins the loaded library.
        unsafe {
            api.cancel.unwrap()(instance, id);
        }
    }
}

/// Returns whether the batch limit left commands that may still be queued.
fn submit_commands(
    api: &abi::PluginApi,
    instance: u64,
    shared: &Shared,
    commands: &Receiver<Command>,
) -> bool {
    // Without room, the consumer's drain notifies the dispatcher again.
    if !shared.room() {
        return false;
    }
    for _ in 0..COMMAND_BATCH {
        let Ok(command) = commands.try_recv() else {
            return false;
        };
        if shared.cancelled(command.id) {
            shared.push(
                HostEvent {
                    request_id: command.id,
                    generation: shared.generation,
                    kind: EventKind::Cancelled,
                    fatal: false,
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
        let status = unsafe { api.submit.unwrap()(instance, &request) };
        if status == abi::OK {
            // A cancellation racing this submission is delivered by the next iteration.
            if let Some(request) = lock(&shared.state).requests.get_mut(&command.id) {
                request.submitted = true;
            }
        } else if command.kind == abi::REQUEST_LOCATE {
            shared.push(
                HostEvent {
                    request_id: command.id,
                    generation: shared.generation,
                    kind: EventKind::Failed(messages::Failure {
                        code: status,
                        message: format!("Plugin rejected optional location request ({status})"),
                    }),
                    fatal: false,
                },
                8192,
            );
        } else {
            shared.failure(command.id, format!("Plugin rejected request ({status})"));
        }
    }
    true
}

/// Returns whether the batch limit left an undelivered reply. A QUEUE_FULL reply is
/// retained with its lease and admission and retried on the short fallback.
fn deliver_replies(
    api: &abi::PluginApi,
    instance: u64,
    bridge: &Bridge,
    replies: &Receiver<Reply>,
    pending_reply: &mut Option<PendingReply>,
) -> bool {
    for _ in 0..REPLY_BATCH {
        if let Some(pending) = pending_reply.take() {
            // SAFETY: pending owns the initial content lease until accepted or rejected.
            if !pending.deliver(bridge, |request| unsafe {
                api.submit.unwrap()(instance, request)
            }) {
                *pending_reply = Some(pending);
                return false;
            }
        }
        let Ok(mut reply) = replies.try_recv() else {
            return false;
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
            *pending_reply = Some(PendingReply {
                id: reply.id,
                payload,
                buffer,
            });
        } else {
            bridge.release(buffer.id);
            bridge.services.fetch_sub(1, Ordering::Release);
            bridge.shared.fatal(
                OPEN_REQUEST_ID,
                "Host read service produced an invalid reply".into(),
            );
        }
    }
    true
}

/// Returns whether the batch limit stopped draining while events may remain.
fn drain_events(api: &abi::PluginApi, instance: u64, shared: &Shared) -> bool {
    for _ in 0..EVENT_BATCH {
        // Without room, the consumer's drain notifies the dispatcher again.
        if !shared.room() {
            return false;
        }
        // SAFETY: table of integer/pointer values has valid zeroed representation.
        let mut event: abi::Event = unsafe { std::mem::zeroed() };
        let mut written = 0;
        // SAFETY: exactly one aligned writable Event and count are exclusively borrowed.
        let status = unsafe { api.drain.unwrap()(instance, 1, &mut event, &mut written) };
        if written > 1 {
            // Only one slot was supplied; release what it names, then fail closed.
            // Unreleased plugin buffers would otherwise keep destroy NOT_QUIESCENT.
            release_event_buffers(api, instance, &event);
            shared.fatal(
                OPEN_REQUEST_ID,
                "Plugin drain exceeded supplied event capacity".into(),
            );
            return false;
        }
        if written == 1 {
            receive_event(api, instance, shared, event);
        }
        if status != abi::OK {
            shared.fatal(OPEN_REQUEST_ID, format!("Plugin drain failed ({status})"));
            return false;
        }
        if written == 0 {
            return false;
        }
    }
    true
}

fn services(
    bridge: Arc<Bridge>,
    reader: Reader,
    file: CapturedFile,
    receive: Receiver<Service>,
    send: SyncSender<Reply>,
) {
    let mut reader = Some(reader);
    // Blocks until a service arrives; `Bridge::close` ends the loop after draining.
    while let Ok(service) = receive.recv() {
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
        bridge.shared.dispatcher.notify();
    }
}

fn release_event_buffers(api: &abi::PluginApi, instance: u64, event: &abi::Event) {
    // SAFETY: release exactly the two plugin-owned buffer IDs on backend, even on invalid/stale data.
    unsafe {
        if event.payload.id != 0 {
            api.release_buffer.unwrap()(instance, event.payload.id);
        }
        if event.content.id != 0 && event.content.id != event.payload.id {
            api.release_buffer.unwrap()(instance, event.content.id);
        }
    }
}

fn receive_event(api: &abi::PluginApi, instance: u64, shared: &Shared, event: abi::Event) {
    let result = decode_event(event);
    release_event_buffers(api, instance, &event);
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
            // A plugin's own Failed event ends only its request, never the session.
            shared.push(
                HostEvent {
                    request_id: event.request_id,
                    generation: event.generation,
                    kind,
                    fatal: false,
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
        abi::EVENT_TEXT_POSITION => {
            EventKind::TextPosition(codec::decode(payload).map_err(invalid)?)
        }
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
        EventKind::TextPosition(position) => {
            matches!(shared.mode, viewer::MODE_TEXT | viewer::MODE_SOURCE)
                && position.revision == shared.revision
                && position.source_offset <= shared.length
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Mutex,
        atomic::{AtomicU32, AtomicUsize},
    };

    fn table() -> abi::PluginApi {
        // SAFETY: integer/pointer/optional-function table has an all-zero valid value.
        unsafe { std::mem::zeroed() }
    }

    #[test]
    fn native_cancel_is_delivered_once_and_never_for_an_unsubmitted_command() {
        static CANCELS: Mutex<Vec<(u64, u64)>> = Mutex::new(Vec::new());
        static SUBMITS: AtomicUsize = AtomicUsize::new(0);
        unsafe extern "C" fn cancel(instance: u64, id: u64) -> abi::Status {
            CANCELS.lock().unwrap().push((instance, id));
            abi::OK
        }
        unsafe extern "C" fn submit(_: u64, _: *const abi::Request) -> abi::Status {
            SUBMITS.fetch_add(1, Ordering::Relaxed);
            abi::OK
        }
        let mut api = table();
        api.cancel = Some(cancel);
        api.submit = Some(submit);
        let shared = crate::session::tests::shared();
        for (id, cancelled, submitted) in [(2, true, true), (3, true, false), (4, false, true)] {
            lock(&shared.state).requests.insert(
                id,
                session::RequestState {
                    cancelled,
                    submitted,
                    cancel_delivered: false,
                },
            );
        }
        for _ in 0..3 {
            deliver_cancels(&api, 9, &shared);
        }
        assert_eq!(*CANCELS.lock().unwrap(), [(9, 2)]);
        // The queued command for request 3 is answered without reaching the plugin.
        let (send, commands) = mpsc::sync_channel(1);
        send.send(Command {
            id: 3,
            kind: abi::REQUEST_VIEWPORT,
            payload: Vec::new(),
        })
        .unwrap();
        assert!(!submit_commands(&api, 9, &shared, &commands));
        assert_eq!(SUBMITS.load(Ordering::Relaxed), 0);
        assert!(!lock(&shared.state).requests.contains_key(&3));
        assert!(matches!(
            lock(&shared.state).events.back(),
            Some((
                HostEvent {
                    request_id: 3,
                    kind: EventKind::Cancelled,
                    ..
                },
                _
            ))
        ));
        deliver_cancels(&api, 9, &shared);
        assert_eq!(CANCELS.lock().unwrap().len(), 1);
    }

    #[test]
    fn over_reported_drain_releases_the_supplied_slot_and_fails_closed() {
        static RELEASED: Mutex<Vec<u64>> = Mutex::new(Vec::new());
        static CAPACITY: AtomicU32 = AtomicU32::new(0);
        unsafe extern "C" fn drain(
            _: u64,
            capacity: u32,
            events: *mut abi::Event,
            written: *mut u32,
        ) -> abi::Status {
            CAPACITY.store(capacity, Ordering::Relaxed);
            // SAFETY: the host supplies one writable event and a writable count.
            unsafe {
                (*events).payload.id = 7;
                (*events).content.id = 8;
                *written = 2;
            }
            abi::OK
        }
        unsafe extern "C" fn release(_: u64, id: u64) -> abi::Status {
            RELEASED.lock().unwrap().push(id);
            abi::OK
        }
        let mut api = table();
        api.drain = Some(drain);
        api.release_buffer = Some(release);
        let shared = crate::session::tests::shared();
        assert!(!drain_events(&api, 9, &shared));
        assert_eq!(CAPACITY.load(Ordering::Relaxed), 1);
        assert_eq!(*RELEASED.lock().unwrap(), [7, 8]);
        assert!(shared.closing.load(Ordering::Acquire));
        assert!(lock(&shared.state).fatal.is_some());
    }

    #[test]
    fn batch_limits_report_pending_work_instead_of_waiting_for_the_idle_fallback() {
        static PENDING_EVENTS: AtomicUsize = AtomicUsize::new(EVENT_BATCH + 1);
        unsafe extern "C" fn drain(
            _: u64,
            _: u32,
            events: *mut abi::Event,
            written: *mut u32,
        ) -> abi::Status {
            let available = PENDING_EVENTS
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
                .is_ok();
            // SAFETY: the host supplies one writable event and a writable count.
            unsafe {
                (*events).kind = abi::EVENT_QUIESCENT;
                *written = u32::from(available);
            }
            abi::OK
        }
        unsafe extern "C" fn submit(_: u64, _: *const abi::Request) -> abi::Status {
            abi::OK
        }
        let mut api = table();
        api.drain = Some(drain);
        api.submit = Some(submit);
        let shared = crate::session::tests::shared();
        assert!(
            drain_events(&api, 9, &shared),
            "a full batch may leave events"
        );
        assert!(!drain_events(&api, 9, &shared));
        let (send, commands) = mpsc::sync_channel(COMMAND_BATCH + 1);
        for id in 10..=10 + COMMAND_BATCH as u64 {
            lock(&shared.state)
                .requests
                .insert(id, session::RequestState::default());
            send.try_send(Command {
                id,
                kind: abi::REQUEST_VIEWPORT,
                payload: Vec::new(),
            })
            .unwrap();
        }
        assert!(submit_commands(&api, 9, &shared, &commands));
        assert!(!submit_commands(&api, 9, &shared, &commands));
        assert!(
            lock(&shared.state)
                .requests
                .values()
                .all(|request| request.submitted)
        );
    }

    #[test]
    fn optional_position_is_validated_against_captured_source_and_mode() {
        let mut shared = crate::session::tests::shared();
        let position = viewer::TextPosition {
            revision: 9,
            source_offset: 64,
            line: 100,
            column: 7,
        };
        assert!(valid_revision(&EventKind::TextPosition(position), &shared));
        assert!(!valid_revision(
            &EventKind::TextPosition(viewer::TextPosition {
                revision: 8,
                ..position
            }),
            &shared
        ));
        assert!(!valid_revision(
            &EventKind::TextPosition(viewer::TextPosition {
                source_offset: 65,
                ..position
            }),
            &shared
        ));
        Arc::get_mut(&mut shared).unwrap().mode = viewer::MODE_BINARY;
        assert!(!valid_revision(&EventKind::TextPosition(position), &shared));
    }
    #[test]
    fn document_budget_accounts_for_decoded_block_and_anchor_allocations() {
        use crate::api::document::{BLOCK_PARAGRAPH, Block, DocumentPage, TableCell, TableRow};
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
                    table: Some(TableRow {
                        header: false,
                        cells: vec![
                            TableCell {
                                start: 0,
                                end: 0,
                                alignment: 0
                            };
                            8
                        ],
                    }),
                    strikethrough: None,
                    kind: BLOCK_PARAGRAPH,
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
            charged
                >= 512
                    * (size_of::<Block>()
                        + size_of::<viewer::Anchor>()
                        + 8 * size_of::<TableCell>()),
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
