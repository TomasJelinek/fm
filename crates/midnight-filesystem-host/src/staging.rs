//! Session-owned content staging. Native paths and handles never cross the worker wire.
use memory::{Content, MEMORY_BUDGET, MEMORY_STAGE_LIMIT, MemoryBudget};
use midnight_filesystem_api::{MAX_READ, Message, StagePurpose};
use midnight_worker_platform::spool::{
    PrivateSpoolDirectory, ProcessRootGuard, SPACE_CHECK_INTERVAL, SpoolRoot, SpoolSpace,
    free_space_reserve,
};
use std::{
    collections::{HashMap, VecDeque},
    io,
    path::Path,
    sync::{Arc, Mutex, PoisonError},
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
    /// It starts at zero, so the first append queries the volume, unless the stage was
    /// admitted with a claim (`space::Admission`), whose query already granted some.
    space_budget: u64,
    /// The spool space reserved for a member declared larger than the memory limit,
    /// until the stage writes it, seals or ends (`staging_space.rs`).
    claim: Option<space::Claim>,
    spool: Content,
}
struct Sealed {
    request: u64,
    purpose: StagePurpose,
    spool: Arc<Content>,
}
#[derive(Clone)]
pub(crate) struct Lease {
    spool: Arc<Content>,
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
    /// The verified content's exact length.
    pub(crate) fn length(&self) -> u64 {
        self.length
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
    /// Spool space promised to member stages of every mount that shares `root`.
    claims: Arc<space::Claims>,
    /// A member stage admitted before its StageBegin is processed (`admit_member`),
    /// with the request it belongs to.
    admitted: Option<(u64, space::Admission)>,
    next: u64,
    pending: Option<(u64, Pending)>,
    sealed: HashMap<u64, Sealed>,
    /// Least recently read first, so `last_read` increases towards the back.
    cache: VecDeque<Cached>,
    cache_bytes: u64,
    cache_budget: u64,
    /// Member reads served from or added to the cache by this mount.
    reads: u64,
    /// Largest stage kept in memory (`staging_memory.rs`); tests lower it.
    memory_limit: u64,
    /// The process-wide budget of staged bytes in memory; tests substitute their own.
    memory: &'static MemoryBudget,
    /// The process's cached spool root, forgotten after a spool fails in it.
    shared: Option<&'static RootCache>,
    /// Declared last: the root and every spool of this mount close first.
    process_root: Option<Arc<ProcessRootGuard>>,
}
impl Staging {
    /// Open this user's private spool root from the platform's per-user candidates. A
    /// root that cannot be verified or cannot hold an anonymous spool yields to the
    /// next, so one unusable location never disables staged archive reads. The choice
    /// is made once per process and kept until a spool fails in it (`RootCache`).
    pub(crate) fn open_private() -> io::Result<Self> {
        SHARED_ROOT.open(root::candidates, PrivateSpoolDirectory::space)
    }
    /// Choose the root of `open_private` before a mount needs it (`RootCache::prepare`).
    pub(crate) fn prepare_private() -> io::Result<()> {
        SHARED_ROOT.prepare(root::candidates, PrivateSpoolDirectory::space)
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
        PrivateSpoolDirectory::open(root)
            .map(|root| Self::in_directory(root, Arc::default(), space))
    }
    fn in_directory(
        root: PrivateSpoolDirectory,
        claims: Arc<space::Claims>,
        space: SpaceQuery,
    ) -> Self {
        Self {
            root,
            space,
            claims,
            admitted: None,
            next: 0,
            pending: None,
            sealed: HashMap::new(),
            cache: VecDeque::new(),
            cache_bytes: 0,
            cache_budget: CACHE_BYTES,
            reads: 0,
            memory_limit: MEMORY_STAGE_LIMIT,
            memory: &MEMORY_BUDGET,
            shared: None,
            process_root: None,
        }
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
    /// Tests of spool-file behavior keep no stage in memory.
    #[cfg(test)]
    pub(crate) fn spool_files_only(&mut self) {
        self.memory_limit = 0;
    }
    #[cfg(test)]
    pub(crate) fn set_memory(&mut self, limit: u64, budget: &'static MemoryBudget) {
        self.memory_limit = limit;
        self.memory = budget;
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
                let admission = match self.admitted.take() {
                    Some((admitted, admission)) if admitted == request => Some(admission),
                    stale => {
                        // An admission that its request did not use returns its claim
                        // before this stage claims space.
                        drop(stale);
                        self.admit(purpose, *expected_length)
                            .map_err(|error| failed(self.shared, &self.root, error))?
                    }
                };
                self.next = self.next.checked_add(1).ok_or_else(invalid)?;
                self.pending = Some((
                    self.next,
                    Pending {
                        request,
                        purpose: *purpose,
                        expected: *expected_length,
                        maximum: *max_length,
                        completed: 0,
                        space_budget: admission.as_ref().map_or(0, |admission| admission.budget),
                        claim: admission.map(|admission| admission.claim),
                        // Content declared larger than the memory limit gets a spool
                        // file at once; everything else starts in memory.
                        spool: if expected_length.is_some_and(|length| length > self.memory_limit) {
                            Content::Disk(
                                self.root
                                    .create()
                                    .map_err(|error| failed(self.shared, &self.root, error))?,
                            )
                        } else {
                            Content::memory(self.memory)
                        },
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
                let next_offset =
                    match pending
                        .spool
                        .append_in_memory(*offset, bytes, self.memory_limit)?
                    {
                        Some(next_offset) => next_offset,
                        None => append_to_spool(
                            pending,
                            *offset,
                            bytes,
                            &self.root,
                            self.space,
                            &self.claims,
                        )
                        .map_err(|error| failed(self.shared, &self.root, error))?,
                    };
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
                pending
                    .spool
                    .finish()
                    .map_err(|error| failed(self.shared, &self.root, error))?;
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
    /// Admit the member stage that the StageBegin `message` opens before `nested` handles
    /// it, so that the session can refuse a member that does not fit through cooperative
    /// cancellation (protocol 3) instead of ending the exchange; `nested` then uses this
    /// admission. Call it only for a StageBegin that `cancellation_checkpoint` accepted.
    pub(crate) fn admit_member(&mut self, message: &Message) -> io::Result<()> {
        self.admitted = None;
        if let Message::StageBegin {
            request,
            purpose,
            expected_length,
            ..
        } = message
            && let Some(admission) = self
                .admit(purpose, *expected_length)
                .map_err(|error| failed(self.shared, &self.root, error))?
        {
            self.admitted = Some((*request, admission));
        }
        Ok(())
    }
    /// A member declared larger than the memory limit gets a spool file at once: claim its
    /// declared length on the spool volume first, or refuse it before anything is written
    /// (`staging_space.rs`). Other stages start in memory, and content of unknown length
    /// is checked as it is appended (`append_to_spool`).
    fn admit(
        &self,
        purpose: &StagePurpose,
        expected: Option<u64>,
    ) -> io::Result<Option<space::Admission>> {
        let (StagePurpose::Member { .. }, Some(length)) = (purpose, expected) else {
            return Ok(None);
        };
        if length <= self.memory_limit {
            return Ok(None);
        }
        let volume = query_volume(self.space, &self.root)?;
        self.claims.admit(length, volume).map(Some)
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
/// Write a chunk to the stage's spool file, first moving there what the stage holds in
/// memory when it spills now. Both count against the volume's free-space reserve, and
/// against the space claimed by other member stages that have not written it yet.
fn append_to_spool(
    pending: &mut Pending,
    offset: u64,
    bytes: &[u8],
    root: &PrivateSpoolDirectory,
    query: SpaceQuery,
    claims: &space::Claims,
) -> io::Result<u64> {
    let length = pending.spool.in_memory() + bytes.len() as u64;
    if pending.space_budget < length {
        // Each stage's first write to its spool file, and at least every
        // SPACE_CHECK_INTERVAL bytes, queries the volume: never once per small append.
        let volume = query_volume(query, root)?;
        let own = pending.claim.as_ref().map_or(0, space::Claim::remaining);
        pending.space_budget = space::above_reserve(volume)
            .saturating_sub(claims.others(own))
            .min(SPACE_CHECK_INTERVAL);
        if pending.space_budget < length {
            // Like any other staging failure this ends the exchange, and the session
            // drops the partial stage with its staging.
            return Err(io::Error::new(
                io::ErrorKind::StorageFull,
                "Archive staging stopped before the archive spool volume's free-space \
                 reserve (the larger of 1 GiB and 2% of its capacity), counting the space \
                 that other archive reads in progress still need",
            ));
        }
    }
    pending.space_budget -= length;
    pending.spool.spill(root)?;
    let next = pending.spool.append(offset, bytes)?;
    if let Some(claim) = &mut pending.claim {
        claim.wrote(length);
    }
    Ok(next)
}
/// One query of the spool volume's free space, with a message that names what failed.
fn query_volume(query: SpaceQuery, root: &PrivateSpoolDirectory) -> io::Result<SpoolSpace> {
    query(root).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("Cannot query the archive spool volume's free space: {error}"),
        )
    })
}
/// The spool root of this process: chosen once from the platform's candidates and kept
/// for every later mount, so mounts neither walk the candidates nor create a test spool
/// again. Its mounts share the space claimed in it (`staging_space.rs`). A spool that
/// fails to be created, written or sized in it (`failed`) makes the next mount choose
/// again. A per-process root is never kept: its guard removes it once the last mount
/// using it closes.
pub(crate) struct RootCache(Mutex<Option<(PrivateSpoolDirectory, Arc<space::Claims>)>>);
static SHARED_ROOT: RootCache = RootCache::new();
impl RootCache {
    pub(crate) const fn new() -> Self {
        Self(Mutex::new(None))
    }
    /// Staging in the cached root, or else in the first usable candidate
    /// (`Staging::open_first`). Concurrent mounts wait for one choice.
    pub(crate) fn open(
        &'static self,
        candidates: impl FnOnce() -> Vec<SpoolRoot>,
        space: SpaceQuery,
    ) -> io::Result<Staging> {
        let mut cached = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let mut staging = match cached.as_ref() {
            Some((root, claims)) => Staging::in_directory(root.clone(), claims.clone(), space),
            None => {
                let staging = Staging::open_first(candidates(), space)?;
                if staging.process_root.is_none() {
                    *cached = Some((staging.root.clone(), staging.claims.clone()));
                }
                staging
            }
        };
        staging.shared = Some(self);
        Ok(staging)
    }
    /// Choose the root as a mount does, before one needs it. The test spool that checks
    /// a root can wait seconds for a busy volume's journal, so the application checks
    /// at startup instead of in the first archive it opens; a mount that starts during
    /// the check waits for this choice rather than making its own. A kept root is not
    /// checked again, and a per-process root is not kept.
    pub(crate) fn prepare(
        &'static self,
        candidates: impl FnOnce() -> Vec<SpoolRoot>,
        space: SpaceQuery,
    ) -> io::Result<()> {
        self.open(candidates, space).map(drop)
    }
    /// Forget `root` if it is the cached root, so the next mount checks the candidates.
    fn forget(&self, root: &PrivateSpoolDirectory) {
        let mut cached = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if cached
            .as_ref()
            .is_some_and(|(cached, _)| cached.is_same(root))
        {
            *cached = None;
        }
    }
}
/// A spool failure in `root`: the process chooses its root again at the next mount.
fn failed(shared: Option<&RootCache>, root: &PrivateSpoolDirectory, error: io::Error) -> io::Error {
    if let Some(shared) = shared {
        shared.forget(root);
    }
    error
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
/// below its reserve. Other programs writing to that volume meanwhile are not counted;
/// `append_to_spool` also leaves out the space that other member stages claimed.
fn space_budget(volume: SpoolSpace) -> u64 {
    space::above_reserve(volume).min(SPACE_CHECK_INTERVAL)
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

#[path = "staging_memory.rs"]
mod memory;
#[cfg(test)]
#[path = "staging_memory_tests.rs"]
mod memory_tests;

#[path = "staging_cancel.rs"]
mod cancel;

#[path = "staging_space.rs"]
mod space;

#[cfg(target_os = "linux")]
#[path = "staging_root_linux.rs"]
mod root;
#[cfg(not(target_os = "linux"))]
#[path = "staging_root_other.rs"]
mod root;
