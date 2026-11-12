//! X11/XWayland drag/drop; native Wayland requires a future same-connection seat bridge.
use midnight_core::{
    Cancellation, DragFinish, DragHost, FileClipboard, FileClipboardIntent, FileDragDropService,
    FileDragDropSession, FileDropRequest, FileDropSink, FileDropTarget, FsError,
    MAX_FILE_CLIPBOARD_BYTES, NativeDragWindow, SharedDropTargets,
};
use std::{
    io,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
use x11rb::{
    CURRENT_TIME, NONE,
    connection::{Connection, RequestConnection},
    protocol::{
        Event,
        xproto::{
            self, Atom, AtomEnum, ChangeWindowAttributesAux, ClientMessageEvent,
            ConnectionExt as _, CreateWindowAux, EventMask, PropMode, Property, Window,
            WindowClass,
        },
    },
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

#[path = "file_drag_drop_selection.rs"]
mod selection;
#[path = "file_drag_drop_source.rs"]
mod source;
use selection::{PendingRead, SendTransfer};
use source::Outgoing;

const TRANSFER_TIMEOUT: Duration = Duration::from_secs(5);
const EVENT_BUDGET: usize = 128;

pub struct NativeFileDragDrop;

impl FileDragDropService for crate::LinuxFileSystem {
    fn open_session(
        &self,
        host: DragHost,
        targets: SharedDropTargets,
        sink: Arc<dyn FileDropSink>,
    ) -> Result<Box<dyn FileDragDropSession>, FsError> {
        NativeFileDragDrop.open_session(host, targets, sink)
    }
}

impl FileDragDropService for NativeFileDragDrop {
    fn open_session(
        &self,
        host: DragHost,
        targets: SharedDropTargets,
        sink: Arc<dyn FileDropSink>,
    ) -> Result<Box<dyn FileDragDropSession>, FsError> {
        let NativeDragWindow::X11 { window } = host.native else {
            return Err(FsError::from_io(
                "Linux file drag/drop",
                Path::new(""),
                io::Error::new(
                    io::ErrorKind::Unsupported,
                    "External file dragging requires an X11/XWayland window; native Wayland is unavailable.",
                ),
            ));
        };
        if window == NONE || host.generation == 0 || host.lifetime.is_cancelled() {
            return Err(failure("The native drag/drop window is no longer live."));
        }
        Ok(Box::new(Session::new(host, window, targets, sink)?))
    }
}

fn failure(message: impl std::fmt::Display) -> FsError {
    FsError::from_io(
        "Linux file drag/drop",
        Path::new(""),
        io::Error::other(message.to_string()),
    )
}

struct Atoms {
    aware: Atom,
    proxy: Atom,
    enter: Atom,
    position: Atom,
    status: Atom,
    leave: Atom,
    drop: Atom,
    finished: Atom,
    selection: Atom,
    copy: Atom,
    move_action: Atom,
    action_list: Atom,
    type_list: Atom,
    uri: Atom,
    targets: Atom,
    incr: Atom,
    read: Atom,
}
impl Atoms {
    fn new(c: &RustConnection) -> Result<Self, FsError> {
        let atom = |name: &[u8]| {
            c.intern_atom(false, name)
                .map_err(failure)?
                .reply()
                .map(|r| r.atom)
                .map_err(failure)
        };
        Ok(Self {
            aware: atom(b"XdndAware")?,
            proxy: atom(b"XdndProxy")?,
            enter: atom(b"XdndEnter")?,
            position: atom(b"XdndPosition")?,
            status: atom(b"XdndStatus")?,
            leave: atom(b"XdndLeave")?,
            drop: atom(b"XdndDrop")?,
            finished: atom(b"XdndFinished")?,
            selection: atom(b"XdndSelection")?,
            copy: atom(b"XdndActionCopy")?,
            move_action: atom(b"XdndActionMove")?,
            action_list: atom(b"XdndActionList")?,
            type_list: atom(b"XdndTypeList")?,
            uri: atom(b"text/uri-list")?,
            targets: atom(b"TARGETS")?,
            incr: atom(b"INCR")?,
            read: atom(b"MIDNIGHT_FILE_DROP_READ")?,
        })
    }
}

#[derive(Clone, Debug)]
struct Candidate {
    revision: u64,
    target: FileDropTarget,
}
struct Incoming {
    source: Window,
    supported: bool,
    candidate: Option<Candidate>,
}

struct Session {
    connection: RustConnection,
    root: Window,
    window: Window,
    proxy: Window,
    host: DragHost,
    atoms: Atoms,
    targets: SharedDropTargets,
    sink: Arc<dyn FileDropSink>,
    previous_aware: Option<u32>,
    previous_proxy: Option<u32>,
    incoming: Option<Incoming>,
    read: Option<PendingRead>,
    outgoing: Option<Outgoing>,
    transfers: Vec<SendTransfer>,
    chunk: usize,
    grabbed_pointer: bool,
    grabbed_keyboard: bool,
    last_time: u32,
}

impl Session {
    fn new(
        host: DragHost,
        window: Window,
        targets: SharedDropTargets,
        sink: Arc<dyn FileDropSink>,
    ) -> Result<Self, FsError> {
        let (connection, _) = x11rb::connect(None).map_err(failure)?;
        let root = connection
            .get_geometry(window)
            .map_err(failure)?
            .reply()
            .map_err(failure)?
            .root;
        let atoms = Atoms::new(&connection)?;
        let previous_aware =
            single_property(&connection, window, atoms.aware, AtomEnum::ATOM.into())?;
        let previous_proxy =
            single_property(&connection, window, atoms.proxy, AtomEnum::WINDOW.into())?;
        let proxy = connection.generate_id().map_err(failure)?;
        connection
            .create_window(
                0,
                proxy,
                root,
                0,
                0,
                1,
                1,
                0,
                WindowClass::INPUT_ONLY,
                0,
                &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
            )
            .map_err(failure)?
            .check()
            .map_err(failure)?;
        connection
            .change_window_attributes(
                window,
                &ChangeWindowAttributesAux::new().event_mask(EventMask::STRUCTURE_NOTIFY),
            )
            .map_err(failure)?
            .check()
            .map_err(failure)?;
        let chunk = connection
            .maximum_request_bytes()
            .saturating_sub(128)
            .clamp(1024, 64 * 1024);
        let session = Self {
            connection,
            root,
            window,
            proxy,
            host,
            atoms,
            targets,
            sink,
            previous_aware,
            previous_proxy,
            incoming: None,
            read: None,
            outgoing: None,
            transfers: Vec::new(),
            chunk,
            grabbed_pointer: false,
            grabbed_keyboard: false,
            last_time: CURRENT_TIME,
        };
        for (window, property, type_, value) in [
            (proxy, session.atoms.proxy, AtomEnum::WINDOW, proxy),
            (proxy, session.atoms.aware, AtomEnum::ATOM, 5),
            // Install the proxy first so failure while clearing XdndAware is reversible.
            (window, session.atoms.proxy, AtomEnum::WINDOW, proxy),
        ] {
            session
                .connection
                .change_property32(PropMode::REPLACE, window, property, type_, &[value])
                .map_err(failure)?
                .check()
                .map_err(failure)?;
        }
        // XDND sources check XdndAware on the verified proxy. Clearing the host's
        // advertisement makes older sources that ignore proxies refuse the drop,
        // instead of reaching winit's lossy, UI-thread file parser.
        session
            .connection
            .delete_property(window, session.atoms.aware)
            .map_err(failure)?
            .check()
            .map_err(failure)?;
        session.connection.flush().map_err(failure)?;
        Ok(session)
    }

    fn message(
        &self,
        recipient: Window,
        window: Window,
        type_: Atom,
        data: [u32; 5],
    ) -> Result<(), FsError> {
        // A peer closing its window during a drag is normal. A connection error is not.
        let _ = self
            .connection
            .send_event(
                false,
                recipient,
                EventMask::NO_EVENT,
                ClientMessageEvent::new(32, window, type_, data),
            )
            .map_err(failure)?
            .check();
        Ok(())
    }

    fn candidate_current(&self, candidate: &Candidate) -> bool {
        if self.host.lifetime.is_cancelled() {
            return false;
        }
        self.targets.lock().ok().is_some_and(|targets| {
            targets.host_generation == self.host.generation
                && targets.revision == candidate.revision
                && targets.validate().is_ok()
                && targets.targets.contains(&candidate.target)
        })
    }

    fn candidate_at(&self, packed: u32) -> Option<Candidate> {
        if self.host.lifetime.is_cancelled() {
            return None;
        }
        let (x, y) = unpack_position(packed);
        let point = self
            .connection
            .translate_coordinates(self.root, self.window, x, y)
            .ok()?
            .reply()
            .ok()?;
        if !point.same_screen {
            return None;
        }
        let targets = self.targets.lock().ok()?;
        Some(Candidate {
            revision: targets.revision,
            target: targets.target_at(
                self.host.generation,
                point.dst_x.into(),
                point.dst_y.into(),
            )?,
        })
    }

    fn offers_copy(&self, source: Window, proposed: Atom) -> bool {
        if proposed == self.atoms.copy {
            return true;
        }
        if proposed != self.atoms.move_action {
            return false;
        }
        // File managers commonly propose Move while offering both operations.
        // Select Copy only when its bounded action list explicitly permits it.
        self.connection
            .get_property(false, source, self.atoms.action_list, AtomEnum::ATOM, 0, 16)
            .ok()
            .and_then(|r| r.reply().ok())
            .is_some_and(|reply| {
                reply.bytes_after == 0
                    && reply.type_ == u32::from(AtomEnum::ATOM)
                    && reply
                        .value32()
                        .is_some_and(|mut values| values.any(|atom| atom == self.atoms.copy))
            })
    }

    fn finish_incoming(&mut self, accepted: bool) -> Result<(), FsError> {
        let read = self.read.take();
        let incoming = self.incoming.take();
        self.sink.hover(None);
        let _ = self.connection.delete_property(self.proxy, self.atoms.read);
        if let Some(source) = read
            .map(|read| read.source)
            .or_else(|| incoming.map(|offer| offer.source))
        {
            self.message(
                source,
                source,
                self.atoms.finished,
                [
                    self.window,
                    u32::from(accepted),
                    if accepted { self.atoms.copy } else { NONE },
                    0,
                    0,
                ],
            )?;
        }
        Ok(())
    }

    fn client_message(&mut self, event: ClientMessageEvent) -> Result<(), FsError> {
        if event.format != 32 {
            return Ok(());
        }
        let data = event.data.as_data32();
        if self.source_message(&event, data) {
            return Ok(());
        }
        if event.window != self.window && event.window != self.proxy {
            return Ok(());
        }
        if self.host.lifetime.is_cancelled() {
            return Ok(());
        }
        if event.type_ == self.atoms.enter {
            if self.read.is_some() {
                self.finish_incoming(false)?;
            }
            self.sink.hover(None);
            let supported = data[0] != NONE
                && (data[1] >> 24) >= 3
                && if data[1] & 1 == 0 {
                    data[2..].contains(&self.atoms.uri)
                } else {
                    self.connection
                        .get_property(false, data[0], self.atoms.type_list, AtomEnum::ATOM, 0, 64)
                        .map_err(failure)?
                        .reply()
                        .ok()
                        .is_some_and(|r| {
                            r.bytes_after == 0
                                && r.type_ == u32::from(AtomEnum::ATOM)
                                && r.value32()
                                    .is_some_and(|mut v| v.any(|a| a == self.atoms.uri))
                        })
                };
            self.incoming = Some(Incoming {
                source: data[0],
                supported,
                candidate: None,
            });
        } else if event.type_ == self.atoms.position {
            let Some(incoming) = &self.incoming else {
                return Ok(());
            };
            if incoming.source != data[0] || self.read.is_some() {
                return Ok(());
            }
            let candidate = if incoming.supported && self.offers_copy(data[0], data[4]) {
                self.candidate_at(data[2])
            } else {
                None
            };
            let accepted = candidate.is_some();
            self.sink.hover(candidate.as_ref().map(|c| c.target.pane));
            self.incoming.as_mut().unwrap().candidate = candidate;
            // Always request positions. A zero status rectangle cannot cache a stale panel mapping.
            self.message(
                data[0],
                data[0],
                self.atoms.status,
                [
                    self.window,
                    if accepted { 3 } else { 2 },
                    0,
                    0,
                    if accepted { self.atoms.copy } else { NONE },
                ],
            )?;
        } else if event.type_ == self.atoms.leave {
            if self.incoming.as_ref().is_some_and(|i| i.source == data[0]) {
                self.incoming = None;
                self.read = None;
                self.sink.hover(None);
                let _ = self.connection.delete_property(self.proxy, self.atoms.read);
            }
        } else if event.type_ == self.atoms.drop {
            if self.read.is_some() {
                return Ok(());
            }
            let Some(offer) = &self.incoming else {
                return Ok(());
            };
            if offer.source != data[0] {
                return Ok(());
            }
            let candidate = offer
                .candidate
                .clone()
                .filter(|c| self.candidate_current(c));
            let owner = self
                .connection
                .get_selection_owner(self.atoms.selection)
                .map_err(failure)?
                .reply()
                .map_err(failure)?
                .owner;
            if let Some(candidate) = candidate.filter(|_| owner == data[0]) {
                self.begin_read(data[0], data[2], candidate)?;
            } else {
                self.finish_incoming(false)?;
            }
        }
        Ok(())
    }

    fn event(&mut self, event: Event) -> Result<(), FsError> {
        match &event {
            Event::PropertyNotify(event) => self.last_time = event.time,
            Event::MotionNotify(event) => self.last_time = event.time,
            Event::ButtonRelease(event) => self.last_time = event.time,
            Event::KeyPress(event) => self.last_time = event.time,
            _ => {}
        }
        match event {
            Event::ClientMessage(event) => self.client_message(event)?,
            Event::SelectionRequest(request) if request.selection == self.atoms.selection => {
                self.selection_request(request)?;
            }
            Event::SelectionNotify(event) => self.selection_notify(event)?,
            Event::PropertyNotify(event)
                if event.state == Property::NEW_VALUE
                    && event.window == self.proxy
                    && event.atom == self.atoms.read =>
            {
                self.read_increment()?;
            }
            Event::PropertyNotify(event) if event.state == Property::DELETE => {
                self.send_increment(event)?
            }
            Event::DestroyNotify(event) if event.window == self.window => {
                self.host.lifetime.cancel()
            }
            Event::DestroyNotify(event) => self.transfers.retain(|t| t.requestor != event.window),
            Event::KeyPress(event) => {
                if let Some(outgoing) = &mut self.outgoing
                    && outgoing.escape.contains(&event.detail)
                {
                    outgoing.cancelled = true;
                }
            }
            Event::SelectionClear(event)
                if event.selection == self.atoms.selection
                    && self
                        .connection
                        .get_selection_owner(self.atoms.selection)
                        .map_err(failure)?
                        .reply()
                        .map_err(failure)?
                        .owner
                        != self.proxy =>
            {
                if let Some(outgoing) = &mut self.outgoing {
                    outgoing.cancelled = true;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

impl FileDragDropSession for Session {
    fn service_pending(&mut self) -> Result<(), FsError> {
        if self.read.as_ref().is_some_and(|r| {
            r.started.elapsed() >= TRANSFER_TIMEOUT || !self.candidate_current(&r.candidate)
        }) {
            self.finish_incoming(false)?;
        }
        if self.host.lifetime.is_cancelled() {
            self.incoming = None;
            self.sink.hover(None);
            return Ok(());
        }
        if self
            .incoming
            .as_ref()
            .and_then(|i| i.candidate.as_ref())
            .is_some_and(|candidate| !self.candidate_current(candidate))
        {
            if let Some(incoming) = &mut self.incoming {
                incoming.candidate = None;
            }
            self.sink.hover(None);
        }
        self.transfers
            .retain(|transfer| transfer.updated.elapsed() < TRANSFER_TIMEOUT);
        for _ in 0..EVENT_BUDGET {
            let Some(event) = self.connection.poll_for_event().map_err(failure)? else {
                break;
            };
            self.event(event)?;
        }
        self.connection.flush().map_err(failure)
    }

    fn start_drag(
        &mut self,
        files: &FileClipboard,
        cancel: &Cancellation,
    ) -> Result<DragFinish, FsError> {
        let result = self.run_source(files, cancel);
        self.clear_source();
        result
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.clear_source();
        self.sink.hover(None);
        // Never replace another registration and never assume the UI window survived.
        if single_property(
            &self.connection,
            self.window,
            self.atoms.proxy,
            AtomEnum::WINDOW.into(),
        )
        .ok()
        .flatten()
            == Some(self.proxy)
        {
            for (property, type_, previous) in [
                (self.atoms.aware, AtomEnum::ATOM, self.previous_aware),
                (self.atoms.proxy, AtomEnum::WINDOW, self.previous_proxy),
            ] {
                if let Some(value) = previous {
                    let _ = self.connection.change_property32(
                        PropMode::REPLACE,
                        self.window,
                        property,
                        type_,
                        &[value],
                    );
                } else {
                    let _ = self.connection.delete_property(self.window, property);
                }
            }
        }
        let _ = self.connection.destroy_window(self.proxy);
        // Flushing only sends requests: another connection can still observe our
        // stale proxy after Drop returns. Finish restoration before releasing the
        // backend-owned session, including when the host window is already gone.
        let _ = self.connection.sync();
    }
}

fn single_property(
    c: &RustConnection,
    window: Window,
    atom: Atom,
    type_: Atom,
) -> Result<Option<u32>, FsError> {
    let reply = c
        .get_property(false, window, atom, AtomEnum::ANY, 0, 1)
        .map_err(failure)?
        .reply()
        .map_err(failure)?;
    if reply.type_ == NONE {
        return Ok(None);
    }
    if reply.type_ != type_
        || reply.format != 32
        || reply.value.len() != 4
        || reply.bytes_after != 0
    {
        return Err(failure(
            "An XDND window property has an invalid or oversized encoding.",
        ));
    }
    Ok(reply.value32().and_then(|mut v| v.next()))
}

fn unpack_position(packed: u32) -> (i16, i16) {
    ((packed >> 16) as u16 as i16, packed as u16 as i16)
}

#[cfg(test)]
#[path = "file_drag_drop_tests.rs"]
mod tests;
