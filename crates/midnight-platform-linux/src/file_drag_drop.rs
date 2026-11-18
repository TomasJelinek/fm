//! X11/XWayland drag/drop; native Wayland requires a future same-connection seat bridge.
use midnight_core::{
    Cancellation, DragFinish, DragHost, FileClipboard, FileClipboardIntent, FileDragDropService,
    FileDragDropSession, FileDragDropWaker, FileDropRequest, FileDropSink, FileDropTarget, FsError,
    MAX_FILE_CLIPBOARD_BYTES, NativeDragWindow, SharedDropTargets,
};
use std::{
    io,
    os::fd::AsFd,
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
/// While a drop's destination is shown or its file list is awaited, `service_pending`
/// rechecks them at this interval: no event marks a read's deadline or the UI
/// republishing its drop targets.
const DROP_RECHECK: Duration = Duration::from_millis(10);
/// Fixed refusal reports for a drop that showed the Copy status. The first two are shared
/// with the Windows target; none ever repeats source-supplied data.
const DESTINATION_CHANGED: &str = midnight_core::DROP_DESTINATION_CHANGED;
const SOURCE_LIST_MISSING: &str = midnight_core::DROP_SOURCE_LIST_MISSING;
const LIST_TOO_LARGE: &str = "The dropped file list exceeds the 4 MiB limit.";

pub(crate) struct NativeFileDragDrop;

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
    /// The last XdndStatus told the source that Copy is accepted: a refusal of the
    /// drop is then reported once. A stale `candidate` does not clear it.
    copy_shown: bool,
    /// The packed root coordinates of the last XdndPosition, where the drop lands.
    position: u32,
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
    /// How long a dropped file list may take to arrive (`TRANSFER_TIMEOUT`); tests
    /// shorten it.
    read_timeout: Duration,
    /// Ends `wait_pending`; None when no eventfd could be created, and the session then
    /// offers no waker.
    wake: Option<crate::clipboard::Wake>,
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
            read_timeout: TRANSFER_TIMEOUT,
            wake: crate::clipboard::Wake::new().ok(),
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

    /// The destination of a pending read: its candidate while still current, or else
    /// the target now at the drop position if it is the same pane, navigation and
    /// folder (a republish such as a refresh or resize). None once the window closed or
    /// another destination or none is shown there: the user released over that folder.
    fn read_destination(&self, read: &PendingRead) -> Option<Candidate> {
        if self.candidate_current(&read.candidate) {
            return Some(read.candidate.clone());
        }
        self.candidate_at(read.position).filter(|candidate| {
            let (now, then) = (&candidate.target, &read.candidate.target);
            now.pane == then.pane
                && now.navigation == then.navigation
                && now.directory == then.directory
        })
    }

    /// Refuses the current drop, which showed the Copy status, and tells the user why,
    /// once: `finish_incoming` ends the drop, so later events find nothing to refuse.
    /// The sink only records the report, so this X11 event loop never waits. A closed
    /// window has nobody to tell.
    fn refuse(&mut self, refusal: FsError) -> Result<(), FsError> {
        if !self.host.lifetime.is_cancelled() {
            self.sink.reject(refusal);
        }
        self.finish_incoming(false)
    }

    fn refuse_drop(&mut self, reason: &str) -> Result<(), FsError> {
        self.refuse(failure(reason))
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
                // A new drag began before the dropped one delivered its file list.
                self.refuse_drop(SOURCE_LIST_MISSING)?;
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
                copy_shown: false,
                position: 0,
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
            if let Some(incoming) = &mut self.incoming {
                incoming.candidate = candidate;
                incoming.copy_shown = accepted;
                incoming.position = data[2];
            }
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
            let (copy_shown, position) = (offer.copy_shown, offer.position);
            if !copy_shown {
                // A Move-only offer or a position without a target: the source never
                // saw the Copy status, so there is nothing to explain.
                return self.finish_incoming(false);
            }
            // As the Windows target does at Drop, the drop goes to the target now at the
            // last position; a republish since that status (a refresh) is no refusal.
            let Some(candidate) = self.candidate_at(position) else {
                return self.refuse_drop(DESTINATION_CHANGED);
            };
            let owner = self
                .connection
                .get_selection_owner(self.atoms.selection)
                .map_err(failure)?
                .reply()
                .map_err(failure)?
                .owner;
            if owner != data[0] {
                return self.refuse_drop(SOURCE_LIST_MISSING);
            }
            self.begin_read(data[0], data[2], candidate, position)?;
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

    /// Serve at most `EVENT_BUDGET` queued events. Returns false only when none was
    /// queued: the request buffer is then flushed and x11rb neither holds an event nor
    /// could read one from the socket, so a wait for the socket to become readable cannot
    /// sleep through one. Round trips and a flush that cannot write read the events
    /// that arrive meanwhile into x11rb's queue, so each poll follows a flush.
    fn serve_events(&mut self) -> Result<bool, FsError> {
        for served in 0..EVENT_BUDGET {
            self.connection.flush().map_err(failure)?;
            let Some(event) = self.connection.poll_for_event().map_err(failure)? else {
                return Ok(served > 0);
            };
            self.event(event)?;
        }
        self.connection.flush().map_err(failure)?;
        Ok(true)
    }

    /// The longest wait for input: `timeout`, or `DROP_RECHECK` while a drop's
    /// destination is shown or its file list is awaited.
    fn wait_limit(&self, timeout: Duration) -> Duration {
        let shown = self
            .incoming
            .as_ref()
            .is_some_and(|incoming| incoming.candidate.is_some());
        if shown || self.read.is_some() {
            timeout.min(DROP_RECHECK)
        } else {
            timeout
        }
    }
}

impl FileDragDropSession for Session {
    fn service_pending(&mut self) -> Result<(), FsError> {
        if self.host.lifetime.is_cancelled() {
            // A closed window has nobody to tell: end a pending read silently.
            if self.read.is_some() {
                self.finish_incoming(false)?;
            }
            self.incoming = None;
            self.sink.hover(None);
            return Ok(());
        }
        let read = self.read.as_ref().map(|read| {
            (
                read.started.elapsed() >= self.read_timeout,
                self.read_destination(read),
            )
        });
        match read {
            Some((true, _)) => self.refuse_drop(SOURCE_LIST_MISSING)?,
            Some((false, None)) => self.refuse_drop(DESTINATION_CHANGED)?,
            Some((false, Some(destination))) => {
                if let Some(read) = &mut self.read {
                    read.candidate = destination;
                }
            }
            None => {}
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
        self.serve_events().map(drop)
    }

    fn wait_pending(&mut self, timeout: Duration) -> Result<(), FsError> {
        // Serve the events that x11rb already queued (a round trip reads the events that
        // precede its reply), which poll() on the socket would sleep through. That also
        // covers a closed window's session, whose `service_pending` serves nothing.
        if self.serve_events()? {
            return Ok(());
        }
        crate::clipboard::wait(
            self.wake.as_ref(),
            Some(self.connection.stream().as_fd()),
            self.wait_limit(timeout),
        )
        .map(drop)
        .map_err(|error| FsError {
            operation: "Linux file drag/drop",
            ..error
        })
    }

    fn waker(&self) -> Option<FileDragDropWaker> {
        self.wake.as_ref().map(crate::clipboard::Wake::waker)
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
