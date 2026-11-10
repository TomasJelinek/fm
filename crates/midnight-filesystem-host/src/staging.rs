//! Session-owned content staging. Native paths and handles never cross the worker wire.
use midnight_filesystem_api::{MAX_READ, Message, StagePurpose};
use midnight_worker_platform::spool::{PrivateSpoolDirectory, Spool};
use std::{
    collections::{HashMap, VecDeque},
    io,
    path::Path,
    sync::Arc,
};

const CACHE_ENTRIES: usize = 16;
struct Pending {
    request: u64,
    purpose: StagePurpose,
    expected: Option<u64>,
    maximum: u64,
    completed: u64,
    spool: Spool,
}
struct Sealed {
    request: u64,
    purpose: StagePurpose,
    spool: Arc<Spool>,
}
#[derive(Clone)]
pub(crate) struct Lease {
    spool: Arc<Spool>,
    offset: u64,
    length: u64,
}
impl Lease {
    pub(crate) fn read(&self, offset: u64, length: u32) -> io::Result<Vec<u8>> {
        if length == 0 || length > MAX_READ {
            return Err(invalid());
        }
        let offset = offset.min(self.length);
        let count = (self.length - offset).min(u64::from(length)) as usize;
        let mut bytes = vec![0; count];
        let mut read = 0;
        while read < count {
            let n = self
                .spool
                .read_at(self.offset + offset + read as u64, &mut bytes[read..])?;
            if n == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            read += n;
        }
        Ok(bytes)
    }
}
pub(crate) struct Staging {
    root: PrivateSpoolDirectory,
    next: u64,
    pending: Option<(u64, Pending)>,
    sealed: HashMap<u64, Sealed>,
    cache: VecDeque<(u64, Lease)>,
}
impl Staging {
    pub(crate) fn open(root: &Path) -> io::Result<Self> {
        Ok(Self {
            root: PrivateSpoolDirectory::open(root)?,
            next: 0,
            pending: None,
            sealed: HashMap::new(),
            cache: VecDeque::new(),
        })
    }
    pub(crate) fn cached(&mut self, entry: u64) -> Option<Lease> {
        let index = self.cache.iter().position(|(id, _)| *id == entry)?;
        let item = self.cache.remove(index)?;
        let result = item.1.clone();
        self.cache.push_back(item);
        Some(result)
    }
    /// Return a nested reply, or None for a terminal message handled by the caller.
    pub(crate) fn nested(
        &mut self,
        operation: &Message,
        message: &Message,
    ) -> io::Result<Option<Message>> {
        let request = operation.request();
        if request != message.request() {
            return Err(invalid());
        }
        let reply = match message {
            Message::StageBegin {
                purpose,
                expected_length,
                max_length,
                ..
            } => {
                if self.pending.is_some()
                    || self.sealed.len() >= 2
                    || self
                        .sealed
                        .values()
                        .any(|stage| matches!(stage.purpose, StagePurpose::Member { .. }))
                {
                    return Err(invalid());
                }
                match (operation, purpose) {
                    (Message::Open { .. }, StagePurpose::Scratch) if self.sealed.is_empty() => (),
                    (Message::Read { entry, .. }, StagePurpose::Member { entry: member })
                        if entry == member => {}
                    _ => return Err(invalid()),
                }
                if expected_length.is_some_and(|length| length > *max_length) {
                    return Err(invalid());
                }
                self.next = self.next.checked_add(1).ok_or_else(invalid)?;
                self.pending = Some((
                    self.next,
                    Pending {
                        request,
                        purpose: *purpose,
                        expected: *expected_length,
                        maximum: *max_length,
                        completed: 0,
                        spool: self.root.create()?,
                    },
                ));
                Message::StageCreated {
                    request,
                    stage: self.next,
                }
            }
            Message::StageAppend {
                stage,
                offset,
                bytes,
                ..
            } => {
                let pending = self.pending_mut(*stage, request)?;
                let end = offset.checked_add(bytes.len() as u64).ok_or_else(invalid)?;
                if end > pending.maximum
                    || pending.expected.is_some_and(|expected| end > expected)
                    || bytes.is_empty()
                {
                    return Err(invalid());
                }
                let next_offset = pending.spool.append(*offset, bytes)?;
                Message::StageAck {
                    request,
                    stage: *stage,
                    next_offset,
                }
            }
            Message::StageProgress {
                stage, completed, ..
            } => {
                let pending = self.pending_mut(*stage, request)?;
                if *completed <= pending.completed {
                    return Err(invalid());
                }
                pending.completed = *completed;
                Message::StageProgressAck {
                    request,
                    stage: *stage,
                    completed: *completed,
                }
            }
            Message::StageSeal { stage, length, .. } => {
                let pending = self.pending_mut(*stage, request)?;
                if pending.spool.length() != *length
                    || pending.expected.is_some_and(|expected| expected != *length)
                {
                    return Err(invalid());
                }
                pending.spool.finish()?;
                let (_, pending) = self.pending.take().ok_or_else(invalid)?;
                self.sealed.insert(
                    *stage,
                    Sealed {
                        request,
                        purpose: pending.purpose,
                        spool: Arc::new(pending.spool),
                    },
                );
                Message::StageSealed {
                    request,
                    stage: *stage,
                    length: *length,
                }
            }
            Message::SpoolRead {
                stage,
                offset,
                length,
                ..
            } => {
                let sealed = self.sealed.get(stage).ok_or_else(invalid)?;
                if sealed.purpose != StagePurpose::Scratch
                    || offset
                        .checked_add(u64::from(*length))
                        .is_none_or(|end| end > sealed.spool.length())
                {
                    return Err(invalid());
                }
                let lease = Lease {
                    spool: sealed.spool.clone(),
                    offset: 0,
                    length: sealed.spool.length(),
                };
                Message::SpoolData {
                    request,
                    stage: *stage,
                    bytes: lease.read(*offset, *length)?,
                }
            }
            Message::StageCreated { .. }
            | Message::StageAck { .. }
            | Message::StageSealed { .. }
            | Message::SpoolData { .. }
            | Message::StageProgressAck { .. } => return Err(invalid()),
            _ => return Ok(None),
        };
        Ok(Some(reply))
    }
    fn pending_mut(&mut self, stage: u64, request: u64) -> io::Result<&mut Pending> {
        self.pending
            .as_mut()
            .filter(|(id, pending)| *id == stage && pending.request == request)
            .map(|(_, pending)| pending)
            .ok_or_else(invalid)
    }
    pub(crate) fn terminal(
        &mut self,
        operation: &Message,
        reply: &Message,
    ) -> io::Result<Option<Lease>> {
        if operation.request() != reply.request() || self.pending.is_some() {
            return Err(invalid());
        }
        match (operation, reply) {
            (Message::Open { .. }, Message::Opened { root: 1, .. })
            | (Message::List { .. }, Message::Entries { .. })
                if !self
                    .sealed
                    .values()
                    .any(|stage| matches!(stage.purpose, StagePurpose::Member { .. })) =>
            {
                Ok(None)
            }
            (
                Message::Read {
                    entry: requested, ..
                },
                Message::ContentReady {
                    entry,
                    stage,
                    offset,
                    length,
                    ..
                },
            ) if requested == entry => {
                if self.sealed.iter().any(|(id, sealed)| {
                    *id != *stage && matches!(sealed.purpose, StagePurpose::Member { .. })
                }) {
                    return Err(invalid());
                }
                let sealed = self.sealed.get(stage).ok_or_else(invalid)?;
                if offset
                    .checked_add(*length)
                    .is_none_or(|end| end > sealed.spool.length())
                {
                    return Err(invalid());
                }
                if let StagePurpose::Member { entry: member } = sealed.purpose
                    && (member != *entry
                        || sealed.request != operation.request()
                        || *offset != 0
                        || *length != sealed.spool.length())
                {
                    return Err(invalid());
                }
                let lease = Lease {
                    spool: sealed.spool.clone(),
                    offset: *offset,
                    length: *length,
                };
                if matches!(sealed.purpose, StagePurpose::Member { .. }) {
                    self.sealed.remove(stage);
                }
                self.cache.retain(|(id, _)| id != entry);
                self.cache.push_back((*entry, lease.clone()));
                if self.cache.len() > CACHE_ENTRIES {
                    self.cache.pop_front();
                }
                Ok(Some(lease))
            }
            _ => Err(invalid()),
        }
    }
}
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Invalid archive staging exchange",
    )
}

#[cfg(test)]
#[path = "staging_tests.rs"]
mod tests;
