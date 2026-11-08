use crate::{Context, guard::lock, host::Host, runtime::Runtime};
use midnight_plugin_api::{abi, codec, wire};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

/// Viewer implementations own parsing, indexing and search. All callbacks run
/// on host backend workers. Use Context services instead of private thread pools.
pub trait Viewer: Send + Sync + Sized + 'static {
    const ID: &'static str;
    const VERSION: &'static str;
    /// Construct bounded state only. Defer file work to accepted requests.
    fn create(configuration: &[u8]) -> Result<Self, abi::Status>;
    fn handle(&self, request: Request, context: Context);
}

struct PayloadQuota {
    used: Arc<AtomicUsize>,
    bytes: usize,
}
impl Drop for PayloadQuota {
    fn drop(&mut self) {
        self.used.fetch_sub(self.bytes, Ordering::Release);
    }
}

/// Owned, bounded CBOR input. The SDK accounts for retained wire bytes; plugin
/// caches and decoded models must separately follow their declared memory budget.
pub struct Request {
    kind: u32,
    payload: Box<[u8]>,
    _quota: PayloadQuota,
}

impl Request {
    pub fn kind(&self) -> u32 {
        self.kind
    }
    pub fn decode<T: codec::Model>(&self) -> Result<T, wire::WireError> {
        codec::decode(&self.payload)
    }

    pub(crate) fn new(
        kind: u32,
        payload: &[u8],
        used: Arc<AtomicUsize>,
    ) -> Result<Self, abi::Status> {
        if !matches!(
            kind,
            abi::REQUEST_OPEN
                | abi::REQUEST_VIEWPORT
                | abi::REQUEST_SEARCH
                | abi::REQUEST_CLOSE
                | abi::REQUEST_LOCATE
        ) {
            return Err(abi::UNSUPPORTED);
        }
        if !(kind == abi::REQUEST_CLOSE && payload.is_empty()) {
            wire::validate(payload).map_err(|_| abi::INVALID)?;
        }
        used.try_update(Ordering::Acquire, Ordering::Relaxed, |bytes| {
            bytes
                .checked_add(payload.len())
                .filter(|&bytes| bytes <= 8 * 1024 * 1024)
        })
        .map_err(|_| abi::QUEUE_FULL)?;
        let quota = PayloadQuota {
            used,
            bytes: payload.len(),
        };
        let mut copy = Vec::new();
        copy.try_reserve_exact(payload.len())
            .map_err(|_| abi::QUEUE_FULL)?;
        copy.extend_from_slice(payload);
        Ok(Self {
            kind,
            payload: copy.into_boxed_slice(),
            _quota: quota,
        })
    }
}

pub(crate) trait Handler: Send + Sync {
    fn handle(&self, request: Request, context: Context);
}
impl<T: Viewer> Handler for T {
    fn handle(&self, request: Request, context: Context) {
        Viewer::handle(self, request, context);
    }
}

pub(crate) struct Instance {
    pub(crate) host: Host,
    pub(crate) runtime: Runtime,
    pub(crate) handler: Arc<dyn Handler>,
    pub(crate) input_bytes: Arc<AtomicUsize>,
    pub(crate) gate: Mutex<()>,
    pub(crate) destroyed: AtomicBool,
}

struct Registry {
    instances: HashMap<u64, Arc<Instance>>,
    creating: usize,
    next: u64,
    buffers: Arc<AtomicU64>,
}
static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();

fn registry() -> &'static Mutex<Registry> {
    REGISTRY.get_or_init(|| {
        Mutex::new(Registry {
            instances: HashMap::new(),
            creating: 0,
            next: 1,
            buffers: Arc::new(AtomicU64::new(1)),
        })
    })
}

struct Reservation {
    id: u64,
    pending: bool,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        if self.pending {
            lock(registry()).creating -= 1;
        }
    }
}

pub(crate) fn create<T: Viewer>(host: Host, configuration: &[u8]) -> Result<u64, abi::Status> {
    let (mut reservation, buffers) = {
        let mut registry = lock(registry());
        if registry.instances.len() + registry.creating >= 16 {
            return Err(abi::QUEUE_FULL);
        }
        let id = registry.next;
        registry.next = id.checked_add(1).ok_or(abi::INVALID)?;
        registry.creating += 1;
        (Reservation { id, pending: true }, registry.buffers.clone())
    };
    // Factories and plugin destructors must never run under the registry mutex.
    let handler = Arc::new(T::create(configuration)?);
    let instance = Arc::new(Instance {
        runtime: Runtime::new(host.clone(), buffers),
        host,
        handler,
        input_bytes: Arc::new(AtomicUsize::new(0)),
        gate: Mutex::new(()),
        destroyed: AtomicBool::new(false),
    });
    let mut registry = lock(registry());
    registry.creating -= 1;
    registry.instances.insert(reservation.id, instance);
    reservation.pending = false;
    Ok(reservation.id)
}

pub(crate) fn with_instance<T>(
    id: u64,
    action: impl FnOnce(&Instance) -> Result<T, abi::Status>,
) -> Result<T, abi::Status> {
    let instance = lock(registry())
        .instances
        .get(&id)
        .cloned()
        .ok_or(abi::UNKNOWN_HANDLE)?;
    let _gate = lock(&instance.gate);
    if instance.destroyed.load(Ordering::Acquire) {
        return Err(abi::UNKNOWN_HANDLE);
    }
    action(&instance)
}

pub(crate) fn destroy(id: u64) -> Result<(), abi::Status> {
    with_instance(id, |instance| {
        if !instance.runtime.quiescent() {
            return Err(abi::NOT_QUIESCENT);
        }
        instance.destroyed.store(true, Ordering::Release);
        let removed = lock(registry()).instances.remove(&id);
        drop(removed); // with_instance still owns it until its gate is unlocked.
        Ok(())
    })
}
