//! Native Wayland drag and drop on the application window's own connection.
//!
//! winit owns the `wl_display` and the window's `wl_surface`; this session lends them
//! (guest mode) and keeps its own objects on a private event queue that only the
//! backend actor dispatches: a `wl_seat` binding whose `wl_pointer` sees the same
//! button serials as winit's, and a `wl_data_device` for that seat. Incoming drags
//! are offered as `text/uri-list` and copied like X11 drops; outgoing drags are
//! `wl_data_source` drags started with the serial of the press that began them.
//!
//! The session never holds a prepared read longer than one bounded poll, so winit's
//! own reads are never kept waiting. Requests are sent from the actor thread, which
//! libwayland permits on a shared display.
use super::{DESTINATION_CHANGED, LIST_TOO_LARGE, SOURCE_LIST_MISSING, TRANSFER_TIMEOUT, failure};
use midnight_core::{
    Cancellation, DragFinish, DragHost, FileClipboard, FileClipboardIntent, FileDragDropSession,
    FileDragDropWaker, FileDropRequest, FileDropSink, FileDropTarget, FsError,
    MAX_FILE_CLIPBOARD_BYTES, NativeDragWindow, SharedDropTargets,
};
use rustix::event::{PollFd, PollFlags, poll};
use std::{
    collections::HashMap,
    io::{Read, Write},
    os::fd::{AsFd, OwnedFd},
    sync::Arc,
    time::{Duration, Instant},
};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum,
    backend::{Backend, ObjectId},
    protocol::{
        wl_data_device::{self, WlDataDevice},
        wl_data_device_manager::{DndAction, WlDataDeviceManager},
        wl_data_offer::{self, WlDataOffer},
        wl_data_source::{self, WlDataSource},
        wl_pointer::{self, WlPointer},
        wl_registry::{self, WlRegistry},
        wl_seat::{self, WlSeat},
        wl_surface::WlSurface,
    },
};

/// The only file list format offered and accepted.
pub(super) const URI_LIST: &str = "text/uri-list";
/// The primary button (`BTN_LEFT`); libinput maps a left-handed setup onto it.
const BUTTON_PRIMARY: u32 = 0x110;
/// Offers announced but not yet entered, and their announced types, stay bounded.
const MAX_OFFERS: usize = 8;
const MAX_OFFER_TYPES: usize = 64;
/// Cadence of an outgoing drag's own loop.
const DRAG_POLL: Duration = Duration::from_millis(10);
/// An outgoing drag that the compositor neither drops nor cancels ends after this.
const DRAG_LIMIT: Duration = Duration::from_secs(120);
/// How long a release this window saw may precede the compositor's own report.
const RELEASE_GRACE: Duration = Duration::from_millis(250);

fn wayland_failure(message: impl std::fmt::Display) -> FsError {
    failure(format!("Wayland drag and drop: {message}"))
}

/// Physical window pixels of surface-local coordinates, which are logical.
pub(super) fn physical(x: f64, y: f64, scale_milli: u32) -> Option<(i32, i32)> {
    let scale = f64::from(scale_milli) / 1000.0;
    let (x, y) = (x * scale, y * scale);
    (x.is_finite() && y.is_finite() && scale > 0.0).then(|| {
        (
            x.floor().clamp(-1e9, 1e9) as i32,
            y.floor().clamp(-1e9, 1e9) as i32,
        )
    })
}

/// Whether an offer can be copied here: it carries a URI list, and its source permits
/// Copy (sources before version 3 announce no actions and copy).
pub(super) fn copyable(types: &[String], actions: Option<DndAction>) -> bool {
    types.iter().any(|kind| kind == URI_LIST)
        && actions.is_none_or(|actions| actions.contains(DndAction::Copy))
}

/// What an outgoing drag's source learned from the compositor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Outgoing {
    pub dropped: Option<Instant>,
    pub sent: bool,
    pub finished: bool,
    pub cancelled: bool,
    /// The drop came before the drag's earliest drop time: its data was withheld.
    pub too_soon: bool,
}
impl Outgoing {
    /// The outcome, once there is one. `legacy`: the compositor's data device predates
    /// version 3 and reports no `dnd_finished`, so a sent list is the end.
    pub(super) fn outcome(&self, legacy: bool) -> Option<DragFinish> {
        if self.too_soon && (self.cancelled || self.finished || self.sent) {
            Some(DragFinish::ReleasedTooSoon)
        } else if self.cancelled {
            Some(DragFinish::Cancelled)
        } else if self.finished || (legacy && self.sent) {
            Some(DragFinish::Copied)
        } else {
            None
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Candidate {
    revision: u64,
    target: FileDropTarget,
}

struct Offer {
    types: Vec<String>,
    actions: Option<DndAction>,
    /// The action the compositor chose for this offer (version 3).
    chosen: Option<DndAction>,
}

struct Incoming {
    offer: Option<WlDataOffer>,
    serial: u32,
    position: (f64, f64),
    candidate: Option<Candidate>,
    /// The last answer told the source that Copy is accepted.
    copy_shown: bool,
}

struct State {
    host: DragHost,
    scale_milli: u32,
    targets: SharedDropTargets,
    sink: Arc<dyn FileDropSink>,
    surface: WlSurface,
    seat: Option<WlSeat>,
    pointer: Option<WlPointer>,
    manager: Option<WlDataDeviceManager>,
    device: Option<WlDataDevice>,
    /// The serial of the primary press on this window while it is held.
    press: Option<u32>,
    offers: HashMap<ObjectId, Offer>,
    incoming: Option<Incoming>,
    /// A drop to read, taken by `service_pending` after dispatching.
    dropped: Option<Incoming>,
    outgoing: Option<Outgoing>,
    /// The list an outgoing drag sends, and the earliest time a drop may receive it.
    sending: Option<(Arc<[u8]>, Instant)>,
}

pub(super) struct WaylandSession {
    connection: Connection,
    queue: EventQueue<State>,
    state: State,
    wake: Option<crate::clipboard::Wake>,
}

impl WaylandSession {
    /// # Safety contract
    /// `display` and `surface` are winit's live `wl_display` and the window's
    /// `wl_surface`, valid while the host lifetime runs; the engine closes this
    /// session when that lifetime is cancelled.
    pub(super) fn open(
        host: DragHost,
        targets: SharedDropTargets,
        sink: Arc<dyn FileDropSink>,
    ) -> Result<Self, FsError> {
        let NativeDragWindow::Wayland {
            display,
            surface,
            scale_milli,
        } = host.native
        else {
            return Err(wayland_failure("the window is not a Wayland window"));
        };
        if display == 0 || surface == 0 || host.generation == 0 || host.lifetime.is_cancelled() {
            return Err(wayland_failure("the window is no longer live"));
        }
        // SAFETY: winit's display outlives the window and this session (see above); the
        // guest backend never disconnects it.
        let backend = unsafe { Backend::from_foreign_display(display as *mut _) };
        let connection = Connection::from_backend(backend);
        // SAFETY: the window's live surface proxy; only compared and used as a drag origin.
        let surface_id = unsafe { ObjectId::from_ptr(WlSurface::interface(), surface as *mut _) }
            .map_err(wayland_failure)?;
        let surface = WlSurface::from_id(&connection, surface_id).map_err(wayland_failure)?;
        let mut queue = connection.new_event_queue::<State>();
        let handle = queue.handle();
        connection.display().get_registry(&handle, ());
        let mut state = State {
            host,
            scale_milli,
            targets,
            sink,
            surface,
            seat: None,
            pointer: None,
            manager: None,
            device: None,
            press: None,
            offers: HashMap::new(),
            incoming: None,
            dropped: None,
            outgoing: None,
            sending: None,
        };
        // Globals, then the seat's capabilities and the device: two round trips.
        for _ in 0..2 {
            queue.roundtrip(&mut state).map_err(wayland_failure)?;
        }
        if state.device.is_none() {
            return Err(wayland_failure(
                "the compositor offers no seat with a data device",
            ));
        }
        Ok(Self {
            connection,
            queue,
            state,
            wake: crate::clipboard::Wake::new().ok(),
        })
    }

    fn dispatch(&mut self) -> Result<(), FsError> {
        self.queue
            .dispatch_pending(&mut self.state)
            .map_err(wayland_failure)?;
        self.connection.flush().map_err(wayland_failure)
    }

    /// Waits up to `timeout` for input on the shared connection or a wake, reading
    /// what arrived into the queues. The prepared read lasts this one poll only.
    fn wait(&mut self, timeout: Duration) -> Result<(), FsError> {
        self.dispatch()?;
        let Some(guard) = self.queue.prepare_read() else {
            // Events are already queued: the caller dispatches them.
            return Ok(());
        };
        let readable =
            crate::clipboard::wait(self.wake.as_ref(), Some(guard.connection_fd()), timeout)?;
        if readable {
            // Another reader may have taken the data meanwhile; that is not an error.
            let _ = guard.read();
        }
        Ok(())
    }

    fn receive(&mut self, mut drop: Incoming) -> Result<(), FsError> {
        let Some(offer) = drop.offer.take() else {
            return Ok(());
        };
        let shown = drop.copy_shown;
        let result = self.read_drop(&offer, &drop);
        let chosen = self
            .state
            .offers
            .remove(&offer.id())
            .and_then(|offer| offer.chosen);
        // `finish` is a protocol error unless the compositor settled on Copy.
        if result.is_ok() && offer.version() >= 3 && chosen == Some(DndAction::Copy) {
            offer.finish();
        }
        offer.destroy();
        self.state.sink.hover(None);
        self.connection.flush().map_err(wayland_failure)?;
        match result {
            Ok(()) => Ok(()),
            // Tell the user why only a drop that showed Copy did nothing.
            Err(error) if shown && !self.state.host.lifetime.is_cancelled() => {
                self.state.sink.reject(error);
                Ok(())
            }
            Err(_) => Ok(()),
        }
    }

    fn read_drop(&self, offer: &WlDataOffer, drop: &Incoming) -> Result<(), FsError> {
        let candidate = drop
            .candidate
            .clone()
            .filter(|candidate| self.state.current(candidate, drop.position))
            .ok_or_else(|| failure(DESTINATION_CHANGED))?;
        let (reader, writer) = std::io::pipe().map_err(wayland_failure)?;
        offer.receive(URI_LIST.into(), writer.as_fd());
        self.connection.flush().map_err(wayland_failure)?;
        drop_writer(writer);
        let bytes = read_bounded(reader.into(), TRANSFER_TIMEOUT)?;
        let paths = crate::file_clipboard::decode_uris(&bytes)
            .map_err(|_| failure("The dropped file list is malformed or not local."))?;
        self.state.sink.try_accept(FileDropRequest {
            files: FileClipboard {
                paths,
                intent: FileClipboardIntent::Copy,
            },
            target: candidate.target,
            targets_revision: candidate.revision,
        })
    }

    fn run_source(
        &mut self,
        files: &FileClipboard,
        cancel: &Cancellation,
        earliest_drop: Instant,
    ) -> Result<DragFinish, FsError> {
        files.validate()?;
        if files.intent != FileClipboardIntent::Copy {
            return Err(failure("Native dragging supports Copy only."));
        }
        self.dispatch()?;
        let (Some(manager), Some(device), Some(serial)) = (
            self.state.manager.clone(),
            self.state.device.clone(),
            self.state.press,
        ) else {
            // The button is no longer held on this window: no drag can start.
            return Ok(DragFinish::Cancelled);
        };
        if cancel.is_cancelled() || self.state.host.lifetime.is_cancelled() {
            return Ok(DragFinish::Cancelled);
        }
        let bytes: Arc<[u8]> = crate::file_clipboard::encode_uris(files)?.into();
        let handle = self.queue.handle();
        let source = manager.create_data_source(&handle, ());
        source.offer(URI_LIST.into());
        let legacy = manager.version() < 3;
        if !legacy {
            source.set_actions(DndAction::Copy);
        }
        self.state.outgoing = Some(Outgoing::default());
        self.state.sending = Some((bytes, earliest_drop));
        device.start_drag(Some(&source), &self.state.surface, None, serial);
        let began = Instant::now();
        let mut released: Option<Instant> = None;
        let result = loop {
            if let Err(error) = self.wait(DRAG_POLL).and_then(|()| self.dispatch()) {
                break Err(error);
            }
            let outgoing = self.state.outgoing.unwrap_or_default();
            if let Some(outcome) = outgoing.outcome(legacy) {
                break Ok(outcome);
            }
            if cancel.is_cancelled()
                || self.state.host.lifetime.is_cancelled()
                || began.elapsed() > DRAG_LIMIT
            {
                break Ok(DragFinish::Cancelled);
            }
            // A running drag takes the pointer from this window, which then sees no
            // release. One it sees means the compositor refused to start the drag (a
            // stale serial): give its report a moment, then end.
            if self.state.press.is_none() && outgoing.dropped.is_none() {
                let since = *released.get_or_insert_with(Instant::now);
                if since.elapsed() > RELEASE_GRACE {
                    break Ok(if Instant::now() < earliest_drop {
                        DragFinish::ReleasedTooSoon
                    } else {
                        DragFinish::Cancelled
                    });
                }
            }
            // A drop whose target never finishes is not a confirmed copy.
            if outgoing
                .dropped
                .is_some_and(|dropped| dropped.elapsed() > TRANSFER_TIMEOUT)
            {
                break Ok(if outgoing.too_soon {
                    DragFinish::ReleasedTooSoon
                } else if outgoing.sent {
                    DragFinish::Unconfirmed
                } else {
                    DragFinish::Cancelled
                });
            }
        };
        source.destroy();
        self.state.outgoing = None;
        self.state.sending = None;
        let _ = self.connection.flush();
        result
    }
}

/// Closes this side's write end so the source's end-of-file reaches the reader.
fn drop_writer(writer: std::io::PipeWriter) {
    drop(writer);
}

/// Reads at most `MAX_FILE_CLIPBOARD_BYTES` from `reader` until end of file, giving up
/// after `timeout`. The source process controls what and how fast it writes.
pub(super) fn read_bounded(reader: OwnedFd, timeout: Duration) -> Result<Vec<u8>, FsError> {
    rustix::fs::fcntl_setfl(&reader, rustix::fs::OFlags::NONBLOCK)
        .map_err(|_| failure(SOURCE_LIST_MISSING))?;
    let mut file = std::fs::File::from(reader);
    let deadline = Instant::now() + timeout;
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        match file.read(&mut chunk) {
            Ok(0) => return Ok(bytes),
            Ok(count) => {
                if bytes.len() + count > MAX_FILE_CLIPBOARD_BYTES {
                    return Err(failure(LIST_TOO_LARGE));
                }
                bytes.extend_from_slice(&chunk[..count]);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Err(failure(SOURCE_LIST_MISSING));
                }
                let mut descriptors = [PollFd::new(&file, PollFlags::IN)];
                let timeout =
                    rustix::event::Timespec::try_from(left).unwrap_or(rustix::event::Timespec {
                        tv_sec: 5,
                        tv_nsec: 0,
                    });
                let _ = poll(&mut descriptors, Some(&timeout));
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return Err(failure(SOURCE_LIST_MISSING)),
        }
    }
}

/// Writes `bytes` to a target's pipe, giving up after `timeout`; the target owns the
/// other end and may stop reading.
pub(super) fn write_bounded(fd: OwnedFd, bytes: &[u8], timeout: Duration) -> bool {
    if rustix::fs::fcntl_setfl(&fd, rustix::fs::OFlags::NONBLOCK).is_err() {
        return false;
    }
    let mut file = std::fs::File::from(fd);
    let deadline = Instant::now() + timeout;
    let mut written = 0;
    while written < bytes.len() {
        match file.write(&bytes[written..]) {
            Ok(0) => return false,
            Ok(count) => written += count,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return false;
                }
                let mut descriptors = [PollFd::new(&file, PollFlags::OUT)];
                let timeout =
                    rustix::event::Timespec::try_from(left).unwrap_or(rustix::event::Timespec {
                        tv_sec: 5,
                        tv_nsec: 0,
                    });
                let _ = poll(&mut descriptors, Some(&timeout));
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return false,
        }
    }
    true
}

impl State {
    fn candidate_at(&self, (x, y): (f64, f64)) -> Option<Candidate> {
        if self.host.lifetime.is_cancelled() {
            return None;
        }
        let (x, y) = physical(x, y, self.scale_milli)?;
        let targets = self.targets.lock().ok()?;
        Some(Candidate {
            revision: targets.revision,
            target: targets.target_at(self.host.generation, x, y)?,
        })
    }

    /// The drop still lands on `candidate`: the same published targets, or a republish
    /// with the same pane, navigation and folder at the drop position.
    fn current(&self, candidate: &Candidate, position: (f64, f64)) -> bool {
        self.candidate_at(position).is_some_and(|now| {
            now == *candidate
                || (now.target.pane == candidate.target.pane
                    && now.target.navigation == candidate.target.navigation
                    && now.target.directory == candidate.target.directory)
        })
    }

    /// Answers the source for the incoming drag at its current position.
    fn evaluate(&mut self) {
        let Some(incoming) = self.incoming.as_ref() else {
            return;
        };
        let copy = incoming.offer.as_ref().is_some_and(|offer| {
            self.offers
                .get(&offer.id())
                .is_some_and(|offer| copyable(&offer.types, offer.actions))
        });
        let candidate = copy.then(|| self.candidate_at(incoming.position)).flatten();
        let incoming = self.incoming.as_mut().unwrap();
        let shown = candidate.is_some();
        if let Some(offer) = &incoming.offer
            && (shown != incoming.copy_shown || incoming.candidate.is_none() && shown)
        {
            offer.accept(incoming.serial, shown.then(|| URI_LIST.to_owned()));
            if offer.version() >= 3 {
                let action = if shown {
                    DndAction::Copy
                } else {
                    DndAction::empty()
                };
                offer.set_actions(action, action);
            }
        }
        incoming.copy_shown = shown;
        incoming.candidate = candidate;
        let pane = incoming
            .candidate
            .as_ref()
            .map(|candidate| candidate.target.pane);
        self.sink.hover(pane);
    }

    fn end_incoming(&mut self) {
        if let Some(incoming) = self.incoming.take()
            && let Some(offer) = incoming.offer
        {
            self.offers.remove(&offer.id());
            offer.destroy();
        }
        self.sink.hover(None);
    }
}

impl Dispatch<WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            if interface == WlSeat::interface().name && state.seat.is_none() {
                state.seat = Some(registry.bind(name, version.min(5), handle, ()));
            } else if interface == WlDataDeviceManager::interface().name && state.manager.is_none()
            {
                state.manager = Some(registry.bind(name, version.min(3), handle, ()));
            }
            if state.device.is_none()
                && let (Some(seat), Some(manager)) = (&state.seat, &state.manager)
            {
                state.device = Some(manager.get_data_device(seat, handle, ()));
            }
        }
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(
        state: &mut Self,
        seat: &WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
            && capabilities.contains(wl_seat::Capability::Pointer)
            && state.pointer.is_none()
        {
            state.pointer = Some(seat.get_pointer(handle, ()));
        }
    }
}

impl Dispatch<WlPointer, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_pointer::Event::Button {
            serial,
            button: BUTTON_PRIMARY,
            state: WEnum::Value(pressed),
            ..
        } = event
        {
            state.press = (pressed == wl_pointer::ButtonState::Pressed).then_some(serial);
        }
    }
}

impl Dispatch<WlDataDeviceManager, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlDataDeviceManager,
        _: <WlDataDeviceManager as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlDataDevice, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::DataOffer { id } => {
                if state.offers.len() >= MAX_OFFERS {
                    id.destroy();
                } else {
                    state.offers.insert(
                        id.id(),
                        Offer {
                            types: Vec::new(),
                            actions: None,
                            chosen: None,
                        },
                    );
                }
            }
            wl_data_device::Event::Enter {
                serial,
                surface,
                x,
                y,
                id,
            } => {
                state.end_incoming();
                if surface.id() != state.surface.id() {
                    if let Some(id) = id {
                        state.offers.remove(&id.id());
                        id.destroy();
                    }
                    return;
                }
                state.incoming = Some(Incoming {
                    offer: id,
                    serial,
                    position: (x, y),
                    candidate: None,
                    copy_shown: false,
                });
                state.evaluate();
            }
            wl_data_device::Event::Motion { x, y, .. } => {
                if let Some(incoming) = state.incoming.as_mut() {
                    incoming.position = (x, y);
                    state.evaluate();
                }
            }
            wl_data_device::Event::Leave => state.end_incoming(),
            wl_data_device::Event::Drop => {
                if let Some(incoming) = state.incoming.take() {
                    if incoming.copy_shown {
                        // Its entry stays until the read: it holds the chosen action.
                        state.dropped = Some(incoming);
                    } else if let Some(offer) = incoming.offer {
                        state.offers.remove(&offer.id());
                        offer.destroy();
                    }
                }
                state.sink.hover(None);
            }
            // The clipboard is served elsewhere; release its offer at once.
            wl_data_device::Event::Selection { id: Some(id) } => {
                state.offers.remove(&id.id());
                id.destroy();
            }
            _ => {}
        }
    }

    wayland_client::event_created_child!(State, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, ()),
    ]);
}

impl Dispatch<WlDataOffer, ()> for State {
    fn event(
        state: &mut Self,
        offer: &WlDataOffer,
        event: wl_data_offer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(entry) = state.offers.get_mut(&offer.id()) else {
            return;
        };
        match event {
            wl_data_offer::Event::Offer { mime_type } => {
                if entry.types.len() < MAX_OFFER_TYPES && mime_type.len() <= 256 {
                    entry.types.push(mime_type);
                }
            }
            wl_data_offer::Event::SourceActions {
                source_actions: WEnum::Value(actions),
            } => {
                entry.actions = Some(actions);
                state.evaluate();
            }
            wl_data_offer::Event::Action {
                dnd_action: WEnum::Value(action),
            } => entry.chosen = Some(action),
            _ => {}
        }
    }
}

impl Dispatch<WlDataSource, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlDataSource,
        event: wl_data_source::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(outgoing) = state.outgoing.as_mut() else {
            return;
        };
        match event {
            wl_data_source::Event::Send { mime_type, fd } => {
                let Some((bytes, earliest)) = state.sending.clone() else {
                    return;
                };
                // A drop before the earliest drop time gets an empty list: the target
                // already saw the drop, but receives nothing to copy.
                if outgoing.dropped.is_some_and(|dropped| dropped < earliest)
                    || Instant::now() < earliest
                {
                    outgoing.too_soon = true;
                    drop(fd);
                } else if mime_type == URI_LIST {
                    outgoing.sent = write_bounded(fd, &bytes, TRANSFER_TIMEOUT);
                }
            }
            wl_data_source::Event::DndDropPerformed => {
                let now = Instant::now();
                outgoing.dropped = Some(now);
                if state
                    .sending
                    .as_ref()
                    .is_some_and(|(_, earliest)| now < *earliest)
                {
                    outgoing.too_soon = true;
                }
            }
            wl_data_source::Event::DndFinished => outgoing.finished = true,
            wl_data_source::Event::Cancelled => outgoing.cancelled = true,
            _ => {}
        }
    }
}

impl FileDragDropSession for WaylandSession {
    fn service_pending(&mut self) -> Result<(), FsError> {
        self.dispatch()?;
        if self.state.host.lifetime.is_cancelled() {
            self.state.end_incoming();
            if let Some(mut dropped) = self.state.dropped.take()
                && let Some(offer) = dropped.offer.take()
            {
                offer.destroy();
            }
            return Ok(());
        }
        if let Some(dropped) = self.state.dropped.take() {
            self.receive(dropped)?;
        }
        Ok(())
    }

    fn wait_pending(&mut self, timeout: Duration) -> Result<(), FsError> {
        // While a drag hovers this window, recheck its destination as the UI republishes.
        let limit = if self.state.incoming.is_some() {
            timeout.min(Duration::from_millis(10))
        } else {
            timeout
        };
        self.wait(limit)
    }

    fn waker(&self) -> Option<FileDragDropWaker> {
        self.wake.as_ref().map(crate::clipboard::Wake::waker)
    }

    fn start_drag(
        &mut self,
        files: &FileClipboard,
        cancel: &Cancellation,
        earliest_drop: Instant,
    ) -> Result<DragFinish, FsError> {
        self.run_source(files, cancel, earliest_drop)
    }
}

impl Drop for WaylandSession {
    fn drop(&mut self) {
        self.state.end_incoming();
        if let Some(mut dropped) = self.state.dropped.take()
            && let Some(offer) = dropped.offer.take()
        {
            offer.destroy();
        }
        // The window may be gone: release only this session's own objects.
        if let Some(device) = self.state.device.take()
            && device.version() >= 2
        {
            device.release();
        }
        if let Some(pointer) = self.state.pointer.take()
            && pointer.version() >= 3
        {
            pointer.release();
        }
        if let Some(seat) = self.state.seat.take()
            && seat.version() >= 5
        {
            seat.release();
        }
        let _ = self.connection.flush();
    }
}

#[cfg(test)]
#[path = "file_drag_drop_wayland_tests.rs"]
mod tests;
