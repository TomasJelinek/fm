//! ICCCM selection owner with bounded incremental transfers and timed clipboard reads.

use super::*;
use std::{collections::HashMap, sync::Arc, thread};
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
        self.transfers
            .retain(|transfer| transfer.updated.elapsed() < Duration::from_secs(5));
        for _ in 0..128 {
            let Some(event) = self.connection.poll_for_event().map_err(error)? else {
                break;
            };
            self.event(event)?;
        }
        self.connection.flush().map_err(error)
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
                        self.transfers.swap_remove(index);
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

    #[test]
    #[ignore = "requires an isolated Xvfb DISPLAY and MIDNIGHT_CLIPBOARD_TEST_ISOLATED=1"]
    fn x11_delayed_selection_clear_preserves_reacquired_clipboard() {
        assert_eq!(
            std::env::var("MIDNIGHT_CLIPBOARD_TEST_ISOLATED").as_deref(),
            Ok("1")
        );
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
        assert_eq!(
            std::env::var("MIDNIGHT_CLIPBOARD_TEST_ISOLATED").as_deref(),
            Ok("1")
        );
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
        assert_eq!(
            std::env::var("MIDNIGHT_CLIPBOARD_TEST_ISOLATED").as_deref(),
            Ok("1")
        );
        let files = FileClipboard {
            paths: (0..1000)
                .map(|index| {
                    let mut bytes = format!("/tmp/{index}-{}", "x".repeat(90)).into_bytes();
                    bytes.push(255);
                    PathBuf::from(OsString::from_vec(bytes))
                })
                .collect(),
            intent: FileClipboardIntent::Move,
        };
        let expected = files.clone();
        let (ready, started) = mpsc::sync_channel(1);
        let (stop, stopped) = mpsc::sync_channel(1);
        let owner = thread::spawn(move || {
            let mut clipboard = Clipboard::new().unwrap();
            clipboard.copy(&files).unwrap();
            ready.send(()).unwrap();
            let began = Instant::now();
            while stopped.try_recv().is_err() && began.elapsed() < Duration::from_secs(15) {
                clipboard.service_pending().unwrap();
                thread::sleep(Duration::from_millis(2));
            }
        });
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        let mut reader = Clipboard::new().unwrap();
        let received = reader.read();
        let _ = stop.send(());
        owner.join().unwrap();
        assert_eq!(received.unwrap(), expected);
    }
}
