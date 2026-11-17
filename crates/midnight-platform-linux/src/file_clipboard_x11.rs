//! ICCCM selection owner with bounded incremental transfers and timed clipboard reads.

use super::*;
use crate::clipboard::Wake;
use std::{collections::HashMap, os::fd::AsFd, sync::Arc, thread};
use x11rb::{
    CURRENT_TIME, NONE,
    connection::{Connection, RequestConnection},
    protocol::{
        Event,
        xproto::{
            self, Atom, AtomEnum, ChangeWindowAttributesAux, ConnectionExt as _, CreateWindowAux,
            EventMask, PropMode, Property, SelectionNotifyEvent, SelectionRequestEvent,
            WindowClass,
        },
    },
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

/// Owner events served in one pass; the rest stay queued for the next pass.
const EVENTS_PER_PASS: usize = 128;
/// An incremental transfer whose requestor stops taking chunks is dropped after this.
const TRANSFER_EXPIRY: Duration = Duration::from_secs(5);

struct Transfer {
    requestor: u32,
    property: Atom,
    target: Atom,
    bytes: Arc<[u8]>,
    offset: usize,
    updated: Instant,
}

pub(crate) struct Clipboard {
    connection: RustConnection,
    window: u32,
    clipboard: Atom,
    targets: Atom,
    multiple: Atom,
    atom_pair: Atom,
    incr: Atom,
    read_property: Atom,
    formats: HashMap<&'static str, Atom>,
    data: HashMap<Atom, Arc<[u8]>>,
    owned: Option<FileClipboard>,
    transfers: Vec<Transfer>,
    chunk: usize,
}

impl Clipboard {
    pub(crate) fn read_text(&mut self) -> Result<String, FsError> {
        self.service_pending()?;
        let owner = self.owner()?;
        if owner == NONE || owner == self.window {
            return Err(crate::clipboard_text::error(
                "The clipboard contains no text.",
            ));
        }
        for name in [
            "UTF8_STRING",
            "text/plain;charset=utf-8",
            "text/plain",
            "STRING",
        ] {
            let target = self
                .connection
                .intern_atom(false, name.as_bytes())
                .map_err(error)?
                .reply()
                .map_err(error)?
                .atom;
            if let Some(bytes) = self.read_target(target)? {
                if self.owner()? != owner {
                    return Err(crate::clipboard_text::error(
                        "The clipboard changed while its text was being read.",
                    ));
                }
                return if name == "STRING" {
                    Ok(bytes.into_iter().map(char::from).collect())
                } else {
                    crate::clipboard_text::decode(bytes)
                };
            }
        }
        Err(crate::clipboard_text::error(
            "The clipboard contains no supported text format.",
        ))
    }
    pub(super) fn new() -> Result<Self, FsError> {
        let (connection, screen) = x11rb::connect(None).map_err(error)?;
        let window = connection.generate_id().map_err(error)?;
        connection
            .create_window(
                0,
                window,
                connection.setup().roots[screen].root,
                0,
                0,
                1,
                1,
                0,
                WindowClass::INPUT_ONLY,
                0,
                &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
            )
            .map_err(error)?
            .check()
            .map_err(error)?;
        let atom = |name: &str| {
            connection
                .intern_atom(false, name.as_bytes())
                .map_err(error)?
                .reply()
                .map(|reply| reply.atom)
                .map_err(error)
        };
        let clipboard = atom("CLIPBOARD")?;
        let targets = atom("TARGETS")?;
        let multiple = atom("MULTIPLE")?;
        let atom_pair = atom("ATOM_PAIR")?;
        let incr = atom("INCR")?;
        let read_property = atom("MIDNIGHT_FILE_CLIPBOARD_READ")?;
        let mut formats = HashMap::new();
        for name in [URI, GNOME, KDE] {
            formats.insert(name, atom(name)?);
        }
        let chunk = connection
            .maximum_request_bytes()
            .saturating_sub(128)
            .clamp(1024, 64 * 1024);
        connection.flush().map_err(error)?;
        Ok(Self {
            connection,
            window,
            clipboard,
            targets,
            multiple,
            atom_pair,
            incr,
            read_property,
            formats,
            data: HashMap::new(),
            owned: None,
            transfers: Vec::new(),
            chunk,
        })
    }

    pub(super) fn copy(&mut self, files: &FileClipboard) -> Result<(), FsError> {
        let data = payloads(files)?
            .into_iter()
            .map(|(name, bytes)| (self.formats[name], Arc::from(bytes)))
            .collect();
        self.connection
            .set_selection_owner(self.window, self.clipboard, CURRENT_TIME)
            .map_err(error)?
            .check()
            .map_err(error)?;
        if self.owner()? != self.window {
            return Err(error("The X11 clipboard ownership request was refused."));
        }
        self.data = data;
        self.owned = Some(files.clone());
        self.connection.flush().map_err(error)
    }

    fn owner(&self) -> Result<u32, FsError> {
        self.connection
            .get_selection_owner(self.clipboard)
            .map_err(error)?
            .reply()
            .map(|reply| reply.owner)
            .map_err(error)
    }

    pub(super) fn read(&mut self) -> Result<FileClipboard, FsError> {
        self.service_pending()?;
        let owner = self.owner()?;
        if owner == NONE {
            return Err(error("The X11 clipboard has no owner."));
        }
        if owner == self.window {
            return self
                .owned
                .clone()
                .ok_or_else(|| error("The X11 file clipboard is empty."));
        }
        let files = if let Some(bytes) = self.read_target(self.formats[GNOME])? {
            decode_gnome(&bytes)?
        } else {
            let bytes = self
                .read_target(self.formats[URI])?
                .ok_or_else(|| error("The X11 clipboard contains no supported file list."))?;
            let paths = decode_uris(&bytes)?;
            let effect = self.read_target(self.formats[KDE])?;
            FileClipboard {
                paths,
                intent: decode_kde(effect.as_deref())?,
            }
        };
        if self.owner()? != owner {
            return Err(error(
                "The clipboard changed while its file list was being read.",
            ));
        }
        Ok(files)
    }

    pub(super) fn service_pending(&mut self) -> Result<(), FsError> {
        self.serve_queued().map(drop)
    }

    /// Serve the queued owner events, at most `EVENTS_PER_PASS` of them, after dropping
    /// expired incremental transfers. Returns false only when nothing changed and nothing
    /// is left: then the request buffer is flushed and x11rb neither holds a queued event
    /// nor could read one from the socket, so a wait for the socket to become readable
    /// cannot sleep while an event is already buffered (round trips such as `owner()`
    /// queue the events that arrive before their reply). True means events were served,
    /// transfers expired or events remain, and the caller re-checks its state.
    fn serve_queued(&mut self) -> Result<bool, FsError> {
        let mut changed = self.expire_transfers()?;
        for _ in 0..EVENTS_PER_PASS {
            // Flush first: a flush that cannot write reads, and queues, incoming events.
            self.connection.flush().map_err(error)?;
            let Some(event) = self.connection.poll_for_event().map_err(error)? else {
                return Ok(changed);
            };
            self.event(event)?;
            changed = true;
        }
        self.connection.flush().map_err(error)?;
        Ok(true)
    }

    /// Drop the incremental transfers whose requestor stopped taking chunks; returns
    /// whether any expired.
    fn expire_transfers(&mut self) -> Result<bool, FsError> {
        let mut expired = false;
        let mut index = 0;
        while index < self.transfers.len() {
            if self.transfers[index].updated.elapsed() < TRANSFER_EXPIRY {
                index += 1;
            } else {
                let transfer = self.transfers.swap_remove(index);
                self.release_requestor(transfer.requestor)?;
                expired = true;
            }
        }
        Ok(expired)
    }

    /// Stop receiving the events of a requestor window that `start_transfer` selected,
    /// once no transfer to it remains, so that its later property changes do not wake
    /// this owner. Event masks are per client: the requestor's own selection stays. This
    /// client selects nothing else on a requestor, which is never its own window (see
    /// `request`). A window that is gone already makes this an ignored error.
    fn release_requestor(&self, requestor: u32) -> Result<(), FsError> {
        if self
            .transfers
            .iter()
            .all(|transfer| transfer.requestor != requestor)
        {
            self.connection
                .change_window_attributes(
                    requestor,
                    &ChangeWindowAttributesAux::new().event_mask(EventMask::NO_EVENT),
                )
                .map_err(error)?
                .ignore_error();
        }
        Ok(())
    }

    /// Whether other applications may still need this owner: it owns CLIPBOARD (the only
    /// selection it takes) until a `SelectionClear` from a newer owner, and it finishes the
    /// incremental transfers that started before that until they complete or expire.
    pub(super) fn owns_clipboard(&self) -> bool {
        self.owned.is_some() || !self.transfers.is_empty()
    }

    /// Block until the X11 connection becomes readable, `wake` is signaled or `timeout`
    /// elapses, and serve the owner events that arrived. Events that are already queued
    /// (or that a pass leaves queued) end the wait at once, served, without blocking. An
    /// active incremental transfer shortens the wait to its expiry, which no event marks.
    pub(super) fn wait(&mut self, wake: Option<&Wake>, timeout: Duration) -> Result<(), FsError> {
        if self.serve_queued()? {
            return Ok(());
        }
        let timeout = self.transfers.iter().fold(timeout, |timeout, transfer| {
            timeout.min(TRANSFER_EXPIRY.saturating_sub(transfer.updated.elapsed()))
        });
        let readable =
            crate::clipboard::wait(wake, Some(self.connection.stream().as_fd()), timeout)?;
        if readable {
            self.serve_queued()?;
        }
        Ok(())
    }

    fn event(&mut self, event: Event) -> Result<(), FsError> {
        match event {
            Event::SelectionRequest(request) if request.selection == self.clipboard => {
                self.request(request)?
            }
            Event::SelectionClear(event) if event.selection == self.clipboard => {
                // This event can predate a newer Copy that already reacquired
                // ownership. Do not discard the replacement payload in that case.
                if self.owner()? != self.window {
                    self.owned = None;
                    self.data.clear();
                }
            }
            Event::PropertyNotify(event) if event.state == Property::DELETE => {
                if let Some(index) = self.transfers.iter().position(|transfer| {
                    transfer.requestor == event.window && transfer.property == event.atom
                }) {
                    let transfer = &mut self.transfers[index];
                    let end = (transfer.offset + self.chunk).min(transfer.bytes.len());
                    let sent = self
                        .connection
                        .change_property8(
                            PropMode::REPLACE,
                            transfer.requestor,
                            transfer.property,
                            transfer.target,
                            &transfer.bytes[transfer.offset..end],
                        )
                        .map_err(error)?
                        .check();
                    if sent.is_err() || transfer.offset == transfer.bytes.len() {
                        let transfer = self.transfers.swap_remove(index);
                        self.release_requestor(transfer.requestor)?;
                    } else {
                        transfer.offset = end;
                        transfer.updated = Instant::now();
                    }
                }
            }
            Event::DestroyNotify(event) => self
                .transfers
                .retain(|transfer| transfer.requestor != event.window),
            _ => {}
        }
        Ok(())
    }

    fn request(&mut self, request: SelectionRequestEvent) -> Result<(), FsError> {
        // A requestor is another client's window. Serving a request that names this
        // owner's window would write to it and select, then on release clear, the property
        // events that its incremental reads rely on. Only a misbehaving client sends one;
        // leave it unanswered, because the answer would reach only this owner (a SendEvent
        // without an event mask goes to the window's creator), where a read in progress
        // could take it for the reply it waits for.
        if request.requestor == self.window {
            return Ok(());
        }
        let property = if request.property == NONE {
            request.target
        } else {
            request.property
        };
        let supported = if request.target == self.multiple {
            self.multiple_request(&request, property)?
        } else {
            self.start_transfer(&request, property, request.target)?
        };
        let response = SelectionNotifyEvent {
            response_type: xproto::SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: request.time,
            requestor: request.requestor,
            selection: request.selection,
            target: request.target,
            property: if supported { property } else { NONE },
        };
        // A disappearing requesting window is routine; do not lose clipboard ownership for it.
        let _ = self
            .connection
            .send_event(false, request.requestor, EventMask::NO_EVENT, response)
            .map_err(error)?
            .check();
        Ok(())
    }

    fn multiple_request(
        &mut self,
        request: &SelectionRequestEvent,
        property: Atom,
    ) -> Result<bool, FsError> {
        if request.property == NONE {
            return Ok(false);
        }
        let reply = self
            .connection
            .get_property(false, request.requestor, property, self.atom_pair, 0, 128)
            .map_err(error)?
            .reply();
        let Ok(reply) = reply else {
            return Ok(false);
        };
        if reply.bytes_after != 0 || reply.format != 32 || reply.type_ != self.atom_pair {
            return Ok(false);
        }
        let Some(values) = reply.value32() else {
            return Ok(false);
        };
        let mut pairs: Vec<_> = values.collect();
        if pairs.len() % 2 != 0 {
            return Ok(false);
        }
        for pair in pairs.as_chunks_mut::<2>().0 {
            if pair[0] == self.multiple
                || pair[1] == NONE
                || pair[1] == property
                || !self.start_transfer(request, pair[1], pair[0])?
            {
                pair[1] = NONE;
            }
        }
        Ok(self
            .connection
            .change_property32(
                PropMode::REPLACE,
                request.requestor,
                property,
                self.atom_pair,
                &pairs,
            )
            .map_err(error)?
            .check()
            .is_ok())
    }

    fn start_transfer(
        &mut self,
        request: &SelectionRequestEvent,
        property: Atom,
        target: Atom,
    ) -> Result<bool, FsError> {
        if target == self.targets {
            let mut targets = vec![self.targets, self.multiple];
            targets.extend(self.data.keys().copied());
            return Ok(self
                .connection
                .change_property32(
                    PropMode::REPLACE,
                    request.requestor,
                    property,
                    AtomEnum::ATOM,
                    &targets,
                )
                .map_err(error)?
                .check()
                .is_ok());
        }
        let Some(bytes) = self.data.get(&target).cloned() else {
            return Ok(false);
        };
        if bytes.len() <= self.chunk {
            return Ok(self
                .connection
                .change_property8(
                    PropMode::REPLACE,
                    request.requestor,
                    property,
                    target,
                    &bytes,
                )
                .map_err(error)?
                .check()
                .is_ok());
        }
        if self.transfers.len() >= 16
            || self.transfers.iter().any(|transfer| {
                transfer.requestor == request.requestor && transfer.property == property
            })
        {
            return Ok(false);
        }
        if self
            .connection
            .change_window_attributes(
                request.requestor,
                &ChangeWindowAttributesAux::new()
                    .event_mask(EventMask::PROPERTY_CHANGE | EventMask::STRUCTURE_NOTIFY),
            )
            .map_err(error)?
            .check()
            .is_err()
        {
            return Ok(false);
        }
        if self
            .connection
            .change_property32(
                PropMode::REPLACE,
                request.requestor,
                property,
                self.incr,
                &[bytes.len() as u32],
            )
            .map_err(error)?
            .check()
            .is_err()
        {
            self.release_requestor(request.requestor)?;
            return Ok(false);
        }
        self.transfers.push(Transfer {
            requestor: request.requestor,
            property,
            target,
            bytes,
            offset: 0,
            updated: Instant::now(),
        });
        Ok(true)
    }

    fn read_target(&mut self, target: Atom) -> Result<Option<Vec<u8>>, FsError> {
        self.connection
            .delete_property(self.window, self.read_property)
            .map_err(error)?
            .check()
            .map_err(error)?;
        self.connection
            .convert_selection(
                self.window,
                self.clipboard,
                target,
                self.read_property,
                CURRENT_TIME,
            )
            .map_err(error)?
            .check()
            .map_err(error)?;
        self.connection.flush().map_err(error)?;
        let started = Instant::now();
        loop {
            let event = self.next_event(started)?;
            if let Event::SelectionNotify(reply) = event
                && reply.requestor == self.window
                && reply.selection == self.clipboard
                && reply.target == target
            {
                if reply.property == NONE {
                    return Ok(None);
                }
                if reply.property != self.read_property {
                    return Err(error("The X11 clipboard returned an unexpected property."));
                }
                break;
            }
            self.event(event)?;
        }
        let reply = self.property()?;
        if reply.type_ != self.incr {
            if reply.format != 8 || reply.type_ != target {
                return Err(error(
                    "The X11 clipboard returned an unexpected file-list encoding.",
                ));
            }
            return Ok(Some(reply.value));
        }
        let hinted = reply
            .value32()
            .and_then(|mut values| values.next())
            .ok_or_else(|| error("The X11 incremental transfer header is invalid."))?
            as usize;
        if hinted > MAX_FILE_CLIPBOARD_BYTES {
            return Err(error("The X11 clipboard exceeds its resource budget."));
        }
        let mut bytes = Vec::new();
        loop {
            self.connection.flush().map_err(error)?;
            let event = self.next_event(started)?;
            if let Event::PropertyNotify(event) = event
                && event.window == self.window
                && event.atom == self.read_property
                && event.state == Property::NEW_VALUE
            {
                let reply = self.property()?;
                if reply.type_ != target || reply.format != 8 {
                    return Err(error("The X11 incremental clipboard changed its encoding."));
                }
                if reply.value.is_empty() {
                    return Ok(Some(bytes));
                }
                if bytes.len() + reply.value.len() > MAX_FILE_CLIPBOARD_BYTES {
                    return Err(error("The X11 clipboard exceeds its resource budget."));
                }
                bytes.extend(reply.value);
            } else {
                self.event(event)?;
            }
        }
    }

    fn property(&self) -> Result<xproto::GetPropertyReply, FsError> {
        let reply = self
            .connection
            .get_property(
                true,
                self.window,
                self.read_property,
                AtomEnum::ANY,
                0,
                (MAX_FILE_CLIPBOARD_BYTES / 4 + 1) as u32,
            )
            .map_err(error)?
            .reply()
            .map_err(error)?;
        if reply.bytes_after != 0 || reply.value.len() > MAX_FILE_CLIPBOARD_BYTES {
            return Err(error(
                "The X11 clipboard property exceeds its resource budget.",
            ));
        }
        Ok(reply)
    }

    fn next_event(&self, started: Instant) -> Result<Event, FsError> {
        loop {
            if started.elapsed() > Duration::from_secs(5) {
                return Err(error(
                    "The X11 clipboard owner did not finish its response.",
                ));
            }
            if let Some(event) = self.connection.poll_for_event().map_err(error)? {
                return Ok(event);
            }
            thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for Clipboard {
    fn drop(&mut self) {
        let _ = self.connection.destroy_window(self.window);
        let _ = self.connection.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    /// Elapses only when the event that should end a wait does not.
    const FAILURE_WAIT: Duration = Duration::from_secs(30);

    /// Whether a wait or loop that began at `started` ended well within `FAILURE_WAIT`.
    fn prompt(started: Instant) -> bool {
        started.elapsed() < FAILURE_WAIT / 2
    }

    /// Assert that the test runs against a private X display, and keep that display from
    /// resetting between tests.
    fn isolated_display() {
        assert_eq!(
            std::env::var("MIDNIGHT_CLIPBOARD_TEST_ISOLATED").as_deref(),
            Ok("1")
        );
        crate::clipboard::tests::keep_display_open();
    }

    fn files(path: &str, intent: FileClipboardIntent) -> FileClipboard {
        FileClipboard {
            paths: vec![path.into()],
            intent,
        }
    }

    /// Raw (non-UTF-8) paths whose encoded list exceeds one property chunk, so that the URI
    /// and GNOME targets need incremental transfers.
    fn large_files(name: &str, intent: FileClipboardIntent) -> FileClipboard {
        FileClipboard {
            paths: (0..1000)
                .map(|index| {
                    let mut bytes = format!("/tmp/{name}-{index}-{}", "x".repeat(90)).into_bytes();
                    bytes.push(255);
                    PathBuf::from(OsString::from_vec(bytes))
                })
                .collect(),
            intent,
        }
    }

    /// A new owner of `files` on another thread. It serves every request and chunk from
    /// the event-driven wait, as the session does, until the returned function stops it;
    /// joining the thread then returns the owner. Returns once the owner holds CLIPBOARD.
    fn serving_owner(files: FileClipboard) -> (impl FnOnce(), thread::JoinHandle<Clipboard>) {
        let (ready, holding) = mpsc::sync_channel(1);
        let (stop, stopped) = mpsc::sync_channel(1);
        let wake = Wake::new().unwrap();
        let waker = wake.waker();
        let owner = thread::spawn(move || {
            let mut clipboard = Clipboard::new().unwrap();
            clipboard.copy(&files).unwrap();
            ready.send(()).unwrap();
            let began = Instant::now();
            // A dropped stop function (a failed test) ends the loop as well.
            while stopped.try_recv() == Err(mpsc::TryRecvError::Empty) {
                assert!(prompt(began), "the reader did not finish");
                clipboard.wait(Some(&wake), FAILURE_WAIT / 2).unwrap();
            }
            clipboard
        });
        holding.recv_timeout(FAILURE_WAIT).unwrap();
        let stop = move || {
            let _ = stop.send(());
            waker();
        };
        (stop, owner)
    }

    #[test]
    #[ignore = "requires an isolated Xvfb DISPLAY and MIDNIGHT_CLIPBOARD_TEST_ISOLATED=1"]
    fn x11_selection_clear_ends_the_wait_and_the_ownership() {
        isolated_display();
        let mut clipboard = Clipboard::new().unwrap();
        assert!(!clipboard.owns_clipboard(), "a new window owns nothing");
        clipboard
            .copy(&files("/tmp/owned", FileClipboardIntent::Copy))
            .unwrap();
        assert!(clipboard.owns_clipboard());
        // Another client takes CLIPBOARD while the owner waits on its connection alone:
        // only the SelectionClear on the X11 socket can end the wait.
        let taker = thread::spawn(|| {
            let mut other = Clipboard::new().unwrap();
            other
                .copy(&files("/tmp/other", FileClipboardIntent::Move))
                .unwrap();
            other
        });
        let started = Instant::now();
        while clipboard.owns_clipboard() {
            assert!(prompt(started), "SelectionClear must end the wait");
            clipboard.wait(None, FAILURE_WAIT).unwrap();
        }
        assert!(prompt(started), "SelectionClear must end the wait");
        drop(taker.join().unwrap());
        assert!(
            clipboard.data.is_empty(),
            "the replaced file list is released"
        );
    }

    #[test]
    #[ignore = "requires an isolated Xvfb DISPLAY and MIDNIGHT_CLIPBOARD_TEST_ISOLATED=1"]
    fn x11_event_queued_by_a_round_trip_ends_the_wait_without_socket_input() {
        isolated_display();
        let mut clipboard = Clipboard::new().unwrap();
        let mut other = Clipboard::new().unwrap();
        let first = files("/tmp/first", FileClipboardIntent::Copy);
        clipboard.copy(&first).unwrap();
        // Checked requests: the server sent the SelectionClear before `copy` returned.
        other.copy(&first).unwrap();
        // The reply follows that SelectionClear on the socket, so reading it queues the
        // event in x11rb and leaves the socket empty: poll() alone would sleep through it.
        clipboard
            .connection
            .get_input_focus()
            .unwrap()
            .reply()
            .unwrap();
        let started = Instant::now();
        clipboard.wait(None, FAILURE_WAIT).unwrap();
        assert!(
            prompt(started),
            "an event already read from the socket must end the wait"
        );
        assert!(
            !clipboard.owns_clipboard(),
            "the wait serves that SelectionClear"
        );
    }

    #[test]
    #[ignore = "requires an isolated Xvfb DISPLAY and MIDNIGHT_CLIPBOARD_TEST_ISOLATED=1"]
    fn x11_wake_ends_a_wait_on_the_connection() {
        isolated_display();
        let mut clipboard = Clipboard::new().unwrap();
        clipboard.service_pending().unwrap();
        let wake = Wake::new().unwrap();
        // A wake before the wait and one from another thread during (or before) the next.
        wake.waker()();
        let started = Instant::now();
        clipboard.wait(Some(&wake), FAILURE_WAIT).unwrap();
        assert!(prompt(started), "an earlier wake must end the next wait");
        let waker = wake.waker();
        let other = thread::spawn(move || waker());
        let started = Instant::now();
        clipboard.wait(Some(&wake), FAILURE_WAIT).unwrap();
        assert!(
            prompt(started),
            "a wake from another thread must end the wait"
        );
        other.join().unwrap();
    }

    #[test]
    #[ignore = "requires an isolated Xvfb DISPLAY and MIDNIGHT_CLIPBOARD_TEST_ISOLATED=1"]
    fn x11_delayed_selection_clear_preserves_reacquired_clipboard() {
        isolated_display();
        let mut clipboard = Clipboard::new().unwrap();
        let mut other = Clipboard::new().unwrap();
        let first = FileClipboard {
            paths: vec!["/tmp/first".into()],
            intent: FileClipboardIntent::Copy,
        };
        let replacement = FileClipboard {
            paths: vec!["/tmp/reacquired".into()],
            intent: FileClipboardIntent::Move,
        };
        clipboard.copy(&first).unwrap();
        other.copy(&first).unwrap();
        assert_eq!(clipboard.owner().unwrap(), other.window);
        // Reacquire before pumping the earlier SelectionClear, as two quick user
        // Copy operations can do while another application changes the clipboard.
        clipboard.copy(&replacement).unwrap();
        assert_eq!(clipboard.owner().unwrap(), clipboard.window);
        clipboard.service_pending().unwrap();
        assert_eq!(clipboard.read().unwrap(), replacement);

        other.copy(&first).unwrap();
        assert_eq!(clipboard.owner().unwrap(), other.window);
        clipboard.service_pending().unwrap();
        assert!(clipboard.owned.is_none());
        assert!(clipboard.data.is_empty());
    }

    #[test]
    #[ignore = "requires an isolated Xvfb DISPLAY and MIDNIGHT_CLIPBOARD_TEST_ISOLATED=1"]
    fn x11_event_flood_cannot_bypass_an_expired_clipboard_deadline() {
        isolated_display();
        let clipboard = Clipboard::new().unwrap();
        for _ in 0..128 {
            clipboard
                .connection
                .change_property8(
                    PropMode::REPLACE,
                    clipboard.window,
                    clipboard.read_property,
                    AtomEnum::STRING,
                    b"unrelated event",
                )
                .unwrap()
                .check()
                .unwrap();
        }
        // A round trip ensures the queued native events are ready before the deadline check.
        clipboard
            .connection
            .get_input_focus()
            .unwrap()
            .reply()
            .unwrap();
        assert!(
            clipboard
                .next_event(Instant::now() - Duration::from_secs(6))
                .is_err(),
            "an already queued event must not extend an expired read deadline"
        );
    }

    #[test]
    #[ignore = "requires an isolated Xvfb DISPLAY and MIDNIGHT_CLIPBOARD_TEST_ISOLATED=1"]
    fn x11_native_file_clipboard_transfers_raw_paths_incrementally() {
        isolated_display();
        let files = large_files("raw", FileClipboardIntent::Move);
        let (stop, owner) = serving_owner(files.clone());
        let mut reader = Clipboard::new().unwrap();
        let received = reader.read();
        stop();
        let owner = owner.join().unwrap();
        assert_eq!(received.unwrap(), files);
        // The finished transfer no longer selects the requestor window's events, so its
        // later property changes cannot wake this idle owner.
        assert!(owner.transfers.is_empty());
        let attributes = owner
            .connection
            .get_window_attributes(reader.window)
            .unwrap()
            .reply()
            .unwrap();
        assert_eq!(attributes.your_event_mask, EventMask::NO_EVENT);
    }

    #[test]
    #[ignore = "requires an isolated Xvfb DISPLAY and MIDNIGHT_CLIPBOARD_TEST_ISOLATED=1"]
    fn x11_request_naming_the_owner_window_is_ignored_and_keeps_incremental_reads() {
        isolated_display();
        let mut clipboard = Clipboard::new().unwrap();
        clipboard
            .copy(&large_files("owned", FileClipboardIntent::Copy))
            .unwrap();
        // A misbehaving client names the owner's own window as the requestor of a target
        // that needs an incremental transfer.
        let (foreign, _) = x11rb::connect(None).unwrap();
        let property = foreign
            .intern_atom(false, b"MIDNIGHT_FOREIGN_REQUEST")
            .unwrap()
            .reply()
            .unwrap()
            .atom;
        foreign
            .convert_selection(
                clipboard.window,
                clipboard.clipboard,
                clipboard.formats[URI],
                property,
                CURRENT_TIME,
            )
            .unwrap()
            .check()
            .unwrap();
        // The server sent the SelectionRequest before that checked request returned, so it
        // precedes this reply on the owner's socket and the owner serves it below.
        clipboard
            .connection
            .get_input_focus()
            .unwrap()
            .reply()
            .unwrap();
        clipboard.service_pending().unwrap();
        assert!(
            clipboard.transfers.is_empty(),
            "no transfer to the owner's own window"
        );
        let written = foreign
            .get_property(false, clipboard.window, property, AtomEnum::ANY, 0, 1)
            .unwrap()
            .reply()
            .unwrap();
        assert_eq!(
            written.type_, NONE,
            "nothing is written to the owner's window"
        );
        let attributes = clipboard
            .connection
            .get_window_attributes(clipboard.window)
            .unwrap()
            .reply()
            .unwrap();
        assert_eq!(
            attributes.your_event_mask,
            EventMask::PROPERTY_CHANGE,
            "the owner's window keeps the selection that its reads rely on"
        );
        // An incremental read from a newer owner arrives as property events on that window.
        let newer = large_files("newer", FileClipboardIntent::Move);
        let (stop, owner) = serving_owner(newer.clone());
        let received = clipboard.read();
        stop();
        drop(owner.join().unwrap());
        assert_eq!(received.unwrap(), newer);
    }
}
