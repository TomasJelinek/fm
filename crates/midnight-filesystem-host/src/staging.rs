//! Session-owned content staging. Native paths and handles never cross the worker wire.
use midnight_filesystem_api::{MAX_READ, Message, StagePurpose};
use midnight_worker_platform::spool::{
    PrivateSpoolDirectory, ProcessRootGuard, SPACE_CHECK_INTERVAL, Spool, SpoolRoot, SpoolSpace,
    free_space_reserve,
};
use std::{
    collections::{HashMap, VecDeque},
    io,
    path::Path,
    sync::Arc,
};

const CACHE_ENTRIES: usize = 16;
/// Spool storage of decoded members kept for later reads of the same member. Least
/// recently read members leave first once it is exceeded, except members in active use.
const CACHE_BYTES: u64 = 512 * 1024 * 1024;
/// A member is in active use while it is one of the `ACTIVE_MEMBERS` most recently read
/// members and was read within the mount's last `ACTIVE_READS` member reads. Such a
/// member stays beyond the byte budget, so a sequential reader whose chunks interleave
/// with reads of up to three other members (a copy beside the two thumbnail renders the
/// scheduler admits, a viewer or another copy) never decodes its member again between
/// two chunks. A member whose reader moved on leaves once later reads pass it.
const ACTIVE_MEMBERS: usize = 4;
const ACTIVE_READS: u64 = 64;
struct Pending {
    request: u64,
    purpose: StagePurpose,
    expected: Option<u64>,
    maximum: u64,
    completed: u64,
    /// Bytes this stage may still append before it queries the spool volume again.
    /// It starts at zero, so the first append queries the volume.
    space_budget: u64,
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
    /// Declared after the spool, which closes first: a read can outlive its mount.
    _process_root: Option<Arc<ProcessRootGuard>>,
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
struct Cached {
    entry: u64,
    lease: Lease,
    /// Value of `Staging::reads` at this member's latest read.
    last_read: u64,
}
/// Queries the free space of the volume that holds a private spool root.
type SpaceQuery = fn(&PrivateSpoolDirectory) -> io::Result<SpoolSpace>;
pub(crate) struct Staging {
    root: PrivateSpoolDirectory,
    /// Queries the volume of `root`; tests substitute a fake volume.
    space: SpaceQuery,
    next: u64,
    pending: Option<(u64, Pending)>,
    sealed: HashMap<u64, Sealed>,
    /// Least recently read first, so `last_read` increases towards the back.
    cache: VecDeque<Cached>,
    cache_bytes: u64,
    cache_budget: u64,
    /// Member reads served from or added to the cache by this mount.
    reads: u64,
    /// Declared last: the root and every spool of this mount close first.
    process_root: Option<Arc<ProcessRootGuard>>,
}
impl Staging {
    /// Open this user's private spool root from the platform's per-user candidates. A
    /// root that cannot be verified or cannot hold an anonymous spool yields to the
    /// next, so one unusable location never disables staged archive reads.
    pub(crate) fn open_private() -> io::Result<Self> {
        Self::open_first(root::candidates(), PrivateSpoolDirectory::space)
    }
    /// The first usable candidate whose volume can take at least one check interval
    /// above its free-space reserve, or else the first usable candidate: a volume at
    /// its reserve refuses every member, so it serves only when no other has room.
    fn open_first(
        roots: impl IntoIterator<Item = SpoolRoot>,
        space: SpaceQuery,
    ) -> io::Result<Self> {
        let mut failure = None;
        let mut fallback = None;
        for root in roots {
            // The guard exists before the open, so a closing mount of this process
            // cannot remove the root between its open and the spools created in it.
            let process_root = root.guard();
            let opened = Self::open_root(&root.path, space).and_then(|staging| {
                drop(staging.root.create()?);
                Ok(staging)
            });
            match opened {
                Ok(staging) => {
                    let staging = Self {
                        process_root,
                        ..staging
                    };
                    if space(&staging.root)
                        .is_ok_and(|volume| space_budget(volume) == SPACE_CHECK_INTERVAL)
                    {
                        return Ok(staging);
                    }
                    fallback.get_or_insert(staging);
                }
                Err(error) => {
                    failure = Some(io::Error::new(
                        error.kind(),
                        format!(
                            "Archive spool root {} is unusable: {error}",
                            root.path.display()
                        ),
                    ));
                }
            }
        }
        fallback.ok_or_else(|| {
            failure.unwrap_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "No private per-user archive spool directory is available",
                )
            })
        })
    }
    fn open_root(root: &Path, space: SpaceQuery) -> io::Result<Self> {
        Ok(Self {
            root: PrivateSpoolDirectory::open(root)?,
            space,
            next: 0,
            pending: None,
            sealed: HashMap::new(),
            cache: VecDeque::new(),
            cache_bytes: 0,
            cache_budget: CACHE_BYTES,
            reads: 0,
            process_root: None,
        })
    }
    /// Test fixtures stage in `root` on a fake volume with ample free space, so no
    /// test depends on the free space of the machine's temporary volume.
    #[cfg(test)]
    pub(crate) fn open(root: &Path) -> io::Result<Self> {
        Self::open_root(root, ample_volume)
    }
    #[cfg(test)]
    pub(crate) fn set_space(&mut self, space: SpaceQuery) {
        self.space = space;
    }
    #[cfg(test)]
    pub(crate) fn set_cache_budget(&mut self, bytes: u64) {
        self.cache_budget = bytes;
    }
    pub(crate) fn cached(&mut self, entry: u64) -> Option<Lease> {
        let index = self.cache.iter().position(|cached| cached.entry == entry)?;
        let mut item = self.cache.remove(index)?;
        item.last_read = self.next_read();
        let lease = item.lease.clone();
        self.cache.push_back(item);
        self.trim();
        Some(lease)
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
                        space_budget: 0,
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
                let pending = pending_stage(&mut self.pending, *stage, request)?;
                let length = bytes.len() as u64;
                let end = offset.checked_add(length).ok_or_else(invalid)?;
                if end > pending.maximum
                    || pending.expected.is_some_and(|expected| end > expected)
                    || bytes.is_empty()
                {
                    return Err(invalid());
                }
                if pending.space_budget < length {
                    // Each stage's first append, and at least every SPACE_CHECK_INTERVAL
                    // bytes, queries the volume: never once per small append.
                    let space = (self.space)(&self.root).map_err(|error| {
                        io::Error::new(
                            error.kind(),
                            format!("Cannot query the archive spool volume's free space: {error}"),
                        )
                    })?;
                    pending.space_budget = space_budget(space);
                    if pending.space_budget < length {
                        // Like any other staging failure this ends the exchange, and
                        // the session drops the partial stage with its staging.
                        return Err(io::Error::new(
                            io::ErrorKind::StorageFull,
                            "Archive staging stopped before the archive spool volume's \
                             free-space reserve (the larger of 1 GiB and 2% of its capacity)",
                        ));
                    }
                }
                pending.space_budget -= length;
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
                let pending = pending_stage(&mut self.pending, *stage, request)?;
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
                let pending = pending_stage(&mut self.pending, *stage, request)?;
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
                    _process_root: self.process_root.clone(),
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
        if let Some(index) = self.cache.iter().position(|cached| cached.entry == entry)
            && let Some(replaced) = self.cache.remove(index)
        {
            self.cache_bytes = self.cache_bytes.saturating_sub(replaced.lease.member_bytes);
        }
        self.cache_bytes = self.cache_bytes.saturating_add(lease.member_bytes);
        let last_read = self.next_read();
        self.cache.push_back(Cached {
            entry,
            lease,
            last_read,
        });
        self.trim();
    }
    fn next_read(&mut self) -> u64 {
        self.reads = self.reads.wrapping_add(1);
        self.reads
    }
    /// Evict least recently read members beyond the entry cap, and beyond the byte
    /// budget while the oldest member is not in active use. Recency increases towards
    /// the back, so once the oldest member is active every member is; the member just
    /// read is always active. Dropping an evicted lease releases its anonymous spool
    /// once no read holds it.
    fn trim(&mut self) {
        loop {
            let over_budget = self.cache_bytes > self.cache_budget
                && self.cache.front().is_some_and(|oldest| {
                    self.cache.len() > ACTIVE_MEMBERS
                        || self.reads.wrapping_sub(oldest.last_read) >= ACTIVE_READS
                });
            if self.cache.len() <= CACHE_ENTRIES && !over_budget {
                return;
            }
            let Some(evicted) = self.cache.pop_front() else {
                return;
            };
            self.cache_bytes = self.cache_bytes.saturating_sub(evicted.lease.member_bytes);
        }
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
                    _process_root: self.process_root.clone(),
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
/// The pending stage `stage` of `request`. Borrowing only the pending field lets an
/// append query the spool volume through the root while it holds the stage.
fn pending_stage(
    pending: &mut Option<(u64, Pending)>,
    stage: u64,
    request: u64,
) -> io::Result<&mut Pending> {
    pending
        .as_mut()
        .filter(|(id, pending)| *id == stage && pending.request == request)
        .map(|(_, pending)| pending)
        .ok_or_else(invalid)
}
/// Bytes a stage may append before it queries the spool volume again: at most
/// `SPACE_CHECK_INTERVAL`, and never so much that the volume's free space would fall
/// below its reserve. Other programs writing to that volume meanwhile are not counted.
fn space_budget(space: SpoolSpace) -> u64 {
    space
        .available
        .saturating_sub(free_space_reserve(space.total))
        .min(SPACE_CHECK_INTERVAL)
}
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Invalid archive staging exchange",
    )
}
/// A 1 TiB volume with all of it free: every query grants a whole check interval.
#[cfg(test)]
fn ample_volume(_: &PrivateSpoolDirectory) -> io::Result<SpoolSpace> {
    Ok(SpoolSpace {
        available: 1 << 40,
        total: 1 << 40,
    })
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
