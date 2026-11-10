use super::*;
use crate::adapter::{Record, Target};
use std::{
    collections::HashSet,
    sync::{
        MutexGuard,
        atomic::{AtomicU64, AtomicUsize},
    },
};
pub(super) const MAX_BYTES: usize = 16 * 1024 * 1024;
pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}
pub(super) fn failure(code: ErrorCode) -> Failure {
    Failure::new(code, format!("Network provider request failed ({code:?})."))
}
pub(super) fn stale() -> Failure {
    failure(ErrorCode::StaleReference)
}
pub(super) fn limit() -> Failure {
    failure(ErrorCode::ResourceLimit)
}
pub(super) type Identity = Vec<Target>;
pub(super) fn identity(location: &Location) -> Identity {
    location
        .trail
        .iter()
        .map(|record| record.target.clone())
        .collect()
}
#[derive(Default)]
pub(super) struct Budget {
    bytes: AtomicUsize,
    nodes: AtomicUsize,
    mounts: AtomicUsize,
}
pub(super) fn shared_budget() -> Arc<Budget> {
    static BUDGET: std::sync::OnceLock<Arc<Budget>> = std::sync::OnceLock::new();
    BUDGET.get_or_init(|| Arc::new(Budget::default())).clone()
}
impl Budget {
    pub fn reserve(
        self: &Arc<Self>,
        bytes: usize,
        nodes: usize,
        mounts: usize,
    ) -> Result<Charge, Failure> {
        self.bytes
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|used| *used <= MAX_BYTES)
            })
            .map_err(|_| limit())?;
        if self
            .nodes
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(nodes).filter(|used| *used <= 4096)
            })
            .is_err()
        {
            self.bytes.fetch_sub(bytes, Ordering::AcqRel);
            return Err(limit());
        }
        if self
            .mounts
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(mounts).filter(|used| *used <= 64)
            })
            .is_err()
        {
            self.bytes.fetch_sub(bytes, Ordering::AcqRel);
            self.nodes.fetch_sub(nodes, Ordering::AcqRel);
            return Err(limit());
        }
        Ok(Charge {
            budget: self.clone(),
            bytes,
            nodes,
            mounts,
        })
    }
}
pub(super) struct Charge {
    budget: Arc<Budget>,
    bytes: usize,
    nodes: usize,
    mounts: usize,
}
impl Charge {
    pub fn grow(&mut self, bytes: usize) -> Result<(), Failure> {
        let mut extra = self.budget.reserve(bytes, 0, 0)?;
        self.bytes += bytes;
        extra.bytes = 0;
        Ok(())
    }
}
impl Drop for Charge {
    fn drop(&mut self) {
        self.budget.bytes.fetch_sub(self.bytes, Ordering::AcqRel);
        self.budget.nodes.fetch_sub(self.nodes, Ordering::AcqRel);
        self.budget.mounts.fetch_sub(self.mounts, Ordering::AcqRel);
    }
}
pub(super) struct NodeRecord {
    pub id: u64,
    pub location: Location,
    pub key: Identity,
    pub published: AtomicBool,
    pub _charge: Charge,
}
impl NodeRecord {
    pub fn model(&self) -> Result<Node, Failure> {
        let record = self.location.trail.last().ok_or_else(stale)?;
        Ok(Node {
            id: self.id,
            address: self.location.encode()?,
            name_encoding: NameEncoding::Opaque,
            raw_name: record.target.key.clone(),
            display: record.display.clone(),
            kind: Kind::Directory,
            metadata: Metadata::default(),
        })
    }
}
#[derive(Default)]
pub(super) struct Nodes {
    pub by_id: HashMap<u64, Arc<NodeRecord>>,
    pub by_key: HashMap<Identity, u64>,
}
pub(super) struct Mount {
    pub closed: AtomicBool,
    pub published: AtomicBool,
    pub nodes: Mutex<Nodes>,
    pub _charge: Charge,
}
pub(super) struct State<A: Adapter> {
    pub mounts: HashMap<u64, Arc<Mount>>,
    pub cursors: HashMap<u64, Arc<Cursor<A>>>,
    pub directories: HashMap<Identity, Arc<Directory>>,
}
impl<A: Adapter> Default for State<A> {
    fn default() -> Self {
        Self {
            mounts: HashMap::new(),
            cursors: HashMap::new(),
            directories: HashMap::new(),
        }
    }
}
pub(super) struct NodeTransaction {
    mount: Arc<Mount>,
    pub ids: Vec<u64>,
    committed: bool,
}
impl NodeTransaction {
    pub fn new(mount: Arc<Mount>) -> Self {
        Self {
            mount,
            ids: Vec::new(),
            committed: false,
        }
    }
    pub fn commit(&mut self) {
        let nodes = lock(&self.mount.nodes);
        for id in &self.ids {
            if let Some(node) = nodes.by_id.get(id) {
                node.published.store(true, Ordering::Release);
            }
        }
        self.committed = true;
    }
}
impl Drop for NodeTransaction {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let retired = {
            let mut nodes = lock(&self.mount.nodes);
            let mut retired = Vec::new();
            for id in &self.ids {
                if let Some(node) = nodes.by_id.remove(id) {
                    nodes.by_key.remove(&node.key);
                    retired.push(node);
                }
            }
            // Entry charges retire with the removed nodes. Release their empty
            // table capacity once per rollback, not once per removed entry.
            if !retired.is_empty() {
                nodes.by_id.shrink_to_fit();
                nodes.by_key.shrink_to_fit();
            }
            retired
        };
        drop(retired);
    }
}
static DISCOVERY: AtomicUsize = AtomicUsize::new(0);
pub(super) struct DiscoveryPermit;
impl DiscoveryPermit {
    pub fn acquire() -> Result<Self, Failure> {
        DISCOVERY
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                (used < 2).then_some(used + 1)
            })
            .map_err(|_| failure(ErrorCode::Busy))?;
        Ok(Self)
    }
}
impl Drop for DiscoveryPermit {
    fn drop(&mut self) {
        DISCOVERY.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(super) const CACHE_TTL: Duration = Duration::from_secs(30);
pub(super) struct Snapshot {
    pub records: Vec<Record>,
    pub _charge: Charge,
}
pub(super) struct Cached {
    pub snapshot: Arc<Snapshot>,
    pub created: Instant,
    pub epoch: u64,
}
impl Cached {
    pub fn fresh(&self, now: Instant, epoch: u64) -> bool {
        self.epoch == epoch && now.saturating_duration_since(self.created) < CACHE_TTL
    }
}
pub(super) struct Directory {
    pub epoch: AtomicU64,
    pub cache: Mutex<Option<Cached>>,
    pub _charge: Charge,
}
pub(super) struct Live<A: Adapter> {
    pub cursor: A::Cursor,
    pub _permit: DiscoveryPermit,
}
pub(super) enum Source<A: Adapter> {
    Live(Live<A>),
    Cached {
        snapshot: Arc<Snapshot>,
        index: usize,
    },
    Finished,
}
pub(super) struct CursorData<A: Adapter> {
    pub source: Source<A>,
    pub next: u64,
    pub done: bool,
    pub pending: Option<Pending>,
    pub seen: HashSet<Target>,
    pub accumulated: Vec<Record>,
    pub charge: Option<Charge>,
}
pub(super) struct Pending {
    pub record: Record,
    _charge: Charge,
}
impl Pending {
    pub fn new(record: Record, budget: &Arc<Budget>) -> Result<Self, Failure> {
        // Cached snapshots are shared, but each cursor owns this extra clone.
        let bytes = accounting::record_bytes(&record)?
            .checked_add(size_of::<Self>())
            .ok_or_else(limit)?;
        let charge = budget.reserve(bytes, 0, 0)?;
        Ok(Self {
            record,
            _charge: charge,
        })
    }
}
pub(super) struct Cursor<A: Adapter> {
    pub mount: Arc<Mount>,
    pub node: Arc<NodeRecord>,
    pub directory: Arc<Directory>,
    pub epoch: u64,
    pub closed: AtomicBool,
    pub busy: AtomicBool,
    pub data: Mutex<CursorData<A>>,
    pub _charge: Charge,
}
impl<A: Adapter> Cursor<A> {
    pub fn stale(&self) -> bool {
        self.closed.load(Ordering::Acquire)
            || self.mount.closed.load(Ordering::Acquire)
            || self.directory.epoch.load(Ordering::Acquire) != self.epoch
    }
}
pub(super) struct Busy<A: Adapter>(pub Arc<Cursor<A>>);
impl<A: Adapter> Drop for Busy<A> {
    fn drop(&mut self) {
        self.0.busy.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests;
