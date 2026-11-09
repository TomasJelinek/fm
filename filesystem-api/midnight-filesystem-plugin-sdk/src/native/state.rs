use super::*;
use std::{
    collections::{HashMap, HashSet},
    sync::{
        MutexGuard,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

pub(super) fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value.lock().unwrap_or_else(|error| error.into_inner())
}
pub(super) fn failure(code: ErrorCode) -> Failure {
    Failure::new(
        code,
        match code {
            ErrorCode::Invalid => "Invalid native filesystem request.",
            ErrorCode::Unsupported => "Native filesystem capability is unavailable.",
            ErrorCode::NotFound => "Native filesystem entry was not found.",
            ErrorCode::PermissionDenied => "Native filesystem access was denied.",
            ErrorCode::SourceChanged => "Native filesystem source changed.",
            ErrorCode::Cancelled => "Native filesystem request canceled.",
            ErrorCode::ResourceLimit => "Native filesystem resource budget exhausted.",
            ErrorCode::Io => "Native filesystem I/O failed.",
            ErrorCode::StaleReference => "Native filesystem reference is no longer valid.",
            ErrorCode::Busy => "Native filesystem resource is still in use.",
        },
    )
}
#[derive(Default)]
pub(super) struct Budget(AtomicUsize);
const CACHE_BYTES: usize = 256 * 1024 * 1024;
pub(super) struct Charge {
    budget: Arc<Budget>,
    bytes: usize,
}
impl Charge {
    pub fn new(budget: Arc<Budget>) -> Self {
        Self { budget, bytes: 0 }
    }
    pub fn grow(&mut self, bytes: usize) -> Result<(), Failure> {
        self.budget
            .0
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|used| *used <= CACHE_BYTES)
            })
            .map_err(|_| failure(ErrorCode::ResourceLimit))?;
        self.bytes += bytes;
        Ok(())
    }
}
impl Drop for Charge {
    fn drop(&mut self) {
        self.budget.0.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}
pub(super) struct Record {
    pub id: u64,
    pub path: Arc<PathBuf>,
    pub kind: Kind,
    _charge: Charge,
}
#[derive(Default)]
pub(super) struct Nodes {
    pub by_id: HashMap<u64, Arc<Record>>,
    by_path: HashMap<Arc<PathBuf>, u64>,
}
pub(super) struct Mount {
    pub nodes: Mutex<Nodes>,
    _slot: Slot,
}
impl Mount {
    pub fn new(slot: Slot) -> Self {
        Self {
            nodes: Mutex::default(),
            _slot: slot,
        }
    }
    pub fn bind(
        &self,
        path: PathBuf,
        kind: Kind,
        budget: Arc<Budget>,
    ) -> Result<Arc<Record>, Failure> {
        let mut nodes = lock(&self.nodes);
        if let Some(id) = nodes.by_path.get(&path)
            && let Some(record) = nodes.by_id.get(id)
            && record.kind == kind
        {
            return Ok(record.clone());
        }
        if nodes.by_id.len() >= 1_000_000 {
            return Err(failure(ErrorCode::ResourceLimit));
        }
        let mut charge = Charge::new(budget);
        charge.grow(
            path.capacity()
                .checked_add(256)
                .ok_or_else(|| failure(ErrorCode::ResourceLimit))?,
        )?;
        nodes
            .by_id
            .try_reserve(1)
            .map_err(|_| failure(ErrorCode::ResourceLimit))?;
        nodes
            .by_path
            .try_reserve(1)
            .map_err(|_| failure(ErrorCode::ResourceLimit))?;
        let id = crate::allocate_handle()?;
        let path = Arc::new(path);
        let record = Arc::new(Record {
            id,
            path: path.clone(),
            kind,
            _charge: charge,
        });
        nodes.by_path.insert(path, id);
        nodes.by_id.insert(id, record.clone());
        Ok(record)
    }
}
pub(super) fn model<A: NativeAdapter>(record: &Record, info: NativeInfo) -> Node {
    let name = record.path.file_name().unwrap_or(record.path.as_os_str());
    let (name_encoding, raw_name) = A::name(name);
    Node {
        id: record.id,
        address: A::encode(&record.path),
        name_encoding,
        raw_name,
        display: name.to_string_lossy().into_owned(),
        kind: record.kind,
        metadata: info.metadata,
    }
}
pub(super) struct Cursor<A: NativeAdapter> {
    pub busy: AtomicBool,
    pub data: Mutex<CursorData<A>>,
    pub directory: Arc<PathBuf>,
    pub closed: AtomicBool,
    pub mount: Arc<Mount>,
    pub _slot: Slot,
}
pub(super) struct CursorData<A: NativeAdapter> {
    pub directory: A::Directory,
    pub next: u64,
    pub pending: Option<Node>,
    pub seen: HashSet<u64>,
    pub charge: Charge,
    pub done: bool,
}
pub(super) struct ReadLease<A: NativeAdapter> {
    pub reader: A::Reader,
    pub closed: AtomicBool,
    pub _mount: Arc<Mount>,
    pub _slot: Slot,
}
pub(super) struct State<A: NativeAdapter> {
    pub cursors: HashMap<u64, Arc<Cursor<A>>>,
    pub readers: HashMap<u64, Arc<ReadLease<A>>>,
    pub mounts: HashMap<u64, Arc<Mount>>,
}

// Slots cover both in-flight OS opens and retained resources, and are the last
// fields destroyed after native handles/iterators and their mount leases close.
#[derive(Default)]
pub(super) struct Slots {
    mounts: Arc<AtomicUsize>,
    cursors: Arc<AtomicUsize>,
    readers: Arc<AtomicUsize>,
}
pub(super) struct Slot(Arc<AtomicUsize>);
impl Slots {
    pub fn mount(&self) -> Result<Slot, Failure> {
        Slot::acquire(self.mounts.clone(), 64)
    }
    pub fn cursor(&self) -> Result<Slot, Failure> {
        Slot::acquire(self.cursors.clone(), 32)
    }
    pub fn reader(&self) -> Result<Slot, Failure> {
        Slot::acquire(self.readers.clone(), 64)
    }
}
impl Slot {
    fn acquire(count: Arc<AtomicUsize>, maximum: usize) -> Result<Self, Failure> {
        count
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                (used < maximum).then_some(used + 1)
            })
            .map_err(|_| failure(ErrorCode::ResourceLimit))?;
        Ok(Self(count))
    }
}
impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
impl<A: NativeAdapter> Default for State<A> {
    fn default() -> Self {
        Self {
            mounts: HashMap::new(),
            cursors: HashMap::new(),
            readers: HashMap::new(),
        }
    }
}
