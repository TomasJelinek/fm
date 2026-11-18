//! Bounded ICCCM transfers for XdndSelection, independent of clipboard ownership.
use super::*;
use x11rb::protocol::xproto::{PropertyNotifyEvent, SelectionNotifyEvent, SelectionRequestEvent};

pub(super) struct PendingRead {
    pub source: Window,
    pub candidate: Candidate,
    /// The packed root coordinates of the drop.
    pub position: u32,
    pub started: Instant,
    time: u32,
    incremental: Option<Vec<u8>>,
}

pub(super) struct SendTransfer {
    pub requestor: Window,
    pub updated: Instant,
    property: Atom,
    bytes: Arc<[u8]>,
    offset: usize,
}

impl Session {
    pub(super) fn begin_read(
        &mut self,
        source: Window,
        time: u32,
        candidate: Candidate,
        position: u32,
    ) -> Result<(), FsError> {
        self.connection
            .delete_property(self.proxy, self.atoms.read)
            .map_err(failure)?
            .check()
            .map_err(failure)?;
        self.read = Some(PendingRead {
            source,
            candidate,
            position,
            time,
            started: Instant::now(),
            incremental: None,
        });
        self.connection
            .convert_selection(
                self.proxy,
                self.atoms.selection,
                self.atoms.uri,
                self.atoms.read,
                time,
            )
            .map_err(failure)?
            .check()
            .map_err(failure)?;
        Ok(())
    }

    /// The transferred property, or the fixed reason to refuse the drop: over the byte
    /// budget, or not delivered at all.
    fn read_property(&self) -> Result<xproto::GetPropertyReply, &'static str> {
        let reply = self
            .connection
            .get_property(
                true,
                self.proxy,
                self.atoms.read,
                AtomEnum::ANY,
                0,
                (MAX_FILE_CLIPBOARD_BYTES / 4 + 1) as u32,
            )
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .ok_or(SOURCE_LIST_MISSING)?;
        if reply.bytes_after != 0 || reply.value.len() > MAX_FILE_CLIPBOARD_BYTES {
            return Err(LIST_TOO_LARGE);
        }
        Ok(reply)
    }

    /// Every refusal here ends a read, which only a drop that showed the Copy status
    /// starts, so each is reported once (see `refuse`).
    pub(super) fn selection_notify(&mut self, event: SelectionNotifyEvent) -> Result<(), FsError> {
        let Some(read) = self.read.as_ref() else {
            return Ok(());
        };
        if event.requestor != self.proxy
            || event.selection != self.atoms.selection
            || event.target != self.atoms.uri
            || event.time != read.time
            || read.incremental.is_some()
        {
            return Ok(());
        }
        if event.property != self.atoms.read {
            // The source refused to convert its selection to a file list.
            return self.refuse_drop(SOURCE_LIST_MISSING);
        }
        let reply = match self.read_property() {
            Ok(reply) => reply,
            Err(reason) => return self.refuse_drop(reason),
        };
        if reply.type_ == self.atoms.incr {
            let hint = reply.value32().and_then(|mut values| values.next());
            let Some(hint) = hint.filter(|_| reply.format == 32 && reply.value.len() == 4) else {
                return self.refuse_drop(SOURCE_LIST_MISSING);
            };
            if hint as usize > MAX_FILE_CLIPBOARD_BYTES {
                return self.refuse_drop(LIST_TOO_LARGE);
            }
            if let Some(read) = &mut self.read {
                read.incremental = Some(Vec::new());
            }
            Ok(())
        } else if reply.type_ == self.atoms.uri && reply.format == 8 {
            self.accept_bytes(reply.value)
        } else {
            self.refuse_drop(SOURCE_LIST_MISSING)
        }
    }

    pub(super) fn read_increment(&mut self) -> Result<(), FsError> {
        if !self.read.as_ref().is_some_and(|r| r.incremental.is_some()) {
            return Ok(());
        }
        let reply = match self.read_property() {
            Ok(reply) => reply,
            Err(reason) => return self.refuse_drop(reason),
        };
        if reply.type_ != self.atoms.uri || reply.format != 8 {
            return self.refuse_drop(SOURCE_LIST_MISSING);
        }
        let Some(bytes) = self
            .read
            .as_mut()
            .and_then(|read| read.incremental.as_mut())
        else {
            return Ok(());
        };
        if reply.value.is_empty() {
            let complete = std::mem::take(bytes);
            self.accept_bytes(complete)
        } else if bytes.len().saturating_add(reply.value.len()) > MAX_FILE_CLIPBOARD_BYTES {
            self.refuse_drop(LIST_TOO_LARGE)
        } else {
            bytes.extend(reply.value);
            Ok(())
        }
    }

    fn accept_bytes(&mut self, bytes: Vec<u8>) -> Result<(), FsError> {
        match self.admit(&bytes) {
            Ok(accepted) => self.finish_incoming(accepted),
            Err(refusal) => self.refuse(refusal),
        }
    }

    /// `Ok(false)` ends silently a read that no longer serves a drop: none is pending,
    /// or the window closed. `Err` is this drop's refusal, in fixed text: the list came
    /// too late or from another selection owner, the drop position no longer shows the
    /// destination (see `read_destination`), or the list or its admission failed.
    fn admit(&self, bytes: &[u8]) -> Result<bool, FsError> {
        let Some(read) = self.read.as_ref() else {
            return Ok(false);
        };
        if self.host.lifetime.is_cancelled() {
            return Ok(false);
        }
        if read.started.elapsed() >= self.read_timeout {
            return Err(failure(SOURCE_LIST_MISSING));
        }
        let candidate = self
            .read_destination(read)
            .ok_or_else(|| failure(DESTINATION_CHANGED))?;
        let owner = self
            .connection
            .get_selection_owner(self.atoms.selection)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|reply| reply.owner);
        if owner != Some(read.source) {
            return Err(failure(SOURCE_LIST_MISSING));
        }
        let request = drop_request(bytes, &candidate)?;
        // This synchronous callback reserves actual backend queue capacity.
        // No native Copy success is sent before it returns successfully.
        self.sink.try_accept(request)?;
        Ok(true)
    }

    pub(super) fn selection_request(
        &mut self,
        request: SelectionRequestEvent,
    ) -> Result<(), FsError> {
        let property = if request.property == NONE {
            request.target
        } else {
            request.property
        };
        let supported = self.start_transfer(&request, property)?;
        let reply = SelectionNotifyEvent {
            response_type: xproto::SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: request.time,
            requestor: request.requestor,
            selection: request.selection,
            target: request.target,
            property: if supported { property } else { NONE },
        };
        let _ = self
            .connection
            .send_event(false, request.requestor, EventMask::NO_EVENT, reply)
            .map_err(failure)?
            .check();
        Ok(())
    }

    fn start_transfer(
        &mut self,
        request: &SelectionRequestEvent,
        property: Atom,
    ) -> Result<bool, FsError> {
        let Some(outgoing) = &self.outgoing else {
            return Ok(false);
        };
        if outgoing.cancelled || self.host.lifetime.is_cancelled() || request.owner != self.proxy {
            return Ok(false);
        }
        if request.target == self.atoms.targets {
            return Ok(self
                .connection
                .change_property32(
                    PropMode::REPLACE,
                    request.requestor,
                    property,
                    AtomEnum::ATOM,
                    &[self.atoms.targets, self.atoms.uri],
                )
                .map_err(failure)?
                .check()
                .is_ok());
        }
        if request.target != self.atoms.uri {
            return Ok(false);
        }
        let bytes = outgoing.bytes.clone();
        if bytes.len() <= self.chunk {
            return Ok(self
                .connection
                .change_property8(
                    PropMode::REPLACE,
                    request.requestor,
                    property,
                    self.atoms.uri,
                    &bytes,
                )
                .map_err(failure)?
                .check()
                .is_ok());
        }
        if self.transfers.len() >= 8
            || self
                .transfers
                .iter()
                .any(|t| t.requestor == request.requestor && t.property == property)
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
            .map_err(failure)?
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
                self.atoms.incr,
                &[bytes.len() as u32],
            )
            .map_err(failure)?
            .check()
            .is_err()
        {
            return Ok(false);
        }
        self.transfers.push(SendTransfer {
            requestor: request.requestor,
            property,
            bytes,
            offset: 0,
            updated: Instant::now(),
        });
        Ok(true)
    }

    pub(super) fn send_increment(&mut self, event: PropertyNotifyEvent) -> Result<(), FsError> {
        let Some(index) = self
            .transfers
            .iter()
            .position(|t| t.requestor == event.window && t.property == event.atom)
        else {
            return Ok(());
        };
        let transfer = &mut self.transfers[index];
        let end = (transfer.offset + self.chunk).min(transfer.bytes.len());
        let sent = self
            .connection
            .change_property8(
                PropMode::REPLACE,
                transfer.requestor,
                transfer.property,
                self.atoms.uri,
                &transfer.bytes[transfer.offset..end],
            )
            .map_err(failure)?
            .check();
        if sent.is_err() || transfer.offset == transfer.bytes.len() {
            self.transfers.swap_remove(index);
        } else {
            transfer.offset = end;
            transfer.updated = Instant::now();
        }
        Ok(())
    }
}

/// The Copy request for a dropped `text/uri-list` aimed at `candidate`. A refusal is
/// shown to the user, so it carries fixed text and never repeats the source's bytes.
pub(super) fn drop_request(
    bytes: &[u8],
    candidate: &Candidate,
) -> Result<FileDropRequest, FsError> {
    // The shared decoder words its errors for the clipboard; one drop message names
    // every condition it rejects.
    let paths = crate::file_clipboard::decode_uris(bytes).map_err(|_| {
        failure(format!(
            "The dropped file list is empty, malformed, names non-local files, or is \
             longer than {} items.",
            midnight_core::MAX_FILE_CLIPBOARD_PATHS
        ))
    })?;
    let request = FileDropRequest {
        files: FileClipboard {
            paths,
            intent: FileClipboardIntent::Copy,
        },
        target: candidate.target.clone(),
        targets_revision: candidate.revision,
    };
    // Core validation reports fixed text of its own.
    request.validate()?;
    Ok(request)
}
