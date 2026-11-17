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
/// Decoded member spools kept for later reads of the same member. Least recently
/// used members leave first; the newest member always stays, so sequential reads
/// of one member larger than this budget still hit its single staged copy.
const CACHE_BYTES: u64 = 512 * 1024 * 1024;
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
    /// Storage only member leases keep allocated; scratch stages stay sealed anyway.
    member_bytes: u64,
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
    cache_bytes: u64,
    cache_budget: u64,
}
impl Staging {
    /// Open this user's private spool root, trying the platform's per-user candidates in
    /// order. A root that cannot be verified or cannot hold an anonymous spool yields to
    /// the next, so one unusable location never disables staged archive reads.
    pub(crate) fn open_private() -> io::Result<Self> {
        Self::open_first(root::candidates())
    }
    fn open_first(roots: impl IntoIterator<Item = std::path::PathBuf>) -> io::Result<Self> {
        let mut failure = None;
        for root in roots {
            let opened = Self::open(&root).and_then(|staging| {
                drop(staging.root.create()?);
                Ok(staging)
            });
            match opened {
                Ok(staging) => return Ok(staging),
                Err(error) => {
                    failure = Some(io::Error::new(
                        error.kind(),
                        format!("Archive spool root {} is unusable: {error}", root.display()),
                    ));
                }
            }
        }
        Err(failure.unwrap_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "No private per-user archive spool directory is available",
            )
        }))
    }
    pub(crate) fn open(root: &Path) -> io::Result<Self> {
        Ok(Self {
            root: PrivateSpoolDirectory::open(root)?,
            next: 0,
            pending: None,
            sealed: HashMap::new(),
            cache: VecDeque::new(),
            cache_bytes: 0,
            cache_budget: CACHE_BYTES,
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
        // Ordinary member writes and cancellation use the same effect-free checks.
        // Scratch preparation during Open retains its separate existing contract.
        self.cancellation_checkpoint(operation, message)?;
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
                    member_bytes: 0,
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
    pub(crate) fn cache_member(&mut self, entry: u64, lease: Lease) {
        if let Some(index) = self.cache.iter().position(|(id, _)| *id == entry)
            && let Some((_, replaced)) = self.cache.remove(index)
        {
            self.cache_bytes = self.cache_bytes.saturating_sub(replaced.member_bytes);
        }
        self.cache_bytes = self.cache_bytes.saturating_add(lease.member_bytes);
        self.cache.push_back((entry, lease));
        // Dropping an evicted lease releases its anonymous spool once no read holds it.
        while self.cache.len() > CACHE_ENTRIES
            || (self.cache_bytes > self.cache_budget && self.cache.len() > 1)
        {
            let Some((_, evicted)) = self.cache.pop_front() else {
                break;
            };
            self.cache_bytes = self.cache_bytes.saturating_sub(evicted.member_bytes);
        }
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
        self.terminal_with_cache(operation, reply, true)
    }
    pub(crate) fn terminal_with_cache(
        &mut self,
        operation: &Message,
        reply: &Message,
        cache: bool,
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
                let member = matches!(sealed.purpose, StagePurpose::Member { .. });
                let lease = Lease {
                    spool: sealed.spool.clone(),
                    offset: *offset,
                    length: *length,
                    member_bytes: if member { sealed.spool.length() } else { 0 },
                };
                if member {
                    self.sealed.remove(stage);
                }
                if cache {
                    self.cache_member(*entry, lease.clone());
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

#[path = "staging_cancel.rs"]
mod cancel;

#[cfg(target_os = "linux")]
#[path = "staging_root_linux.rs"]
mod root;
#[cfg(not(target_os = "linux"))]
#[path = "staging_root_other.rs"]
mod root;
