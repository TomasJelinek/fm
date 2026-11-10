use crate::{
    CallContext, Filesystem, Reply,
    api::{Failure, Request, Response, abi::*},
};
use std::{
    any::TypeId,
    collections::HashMap,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex, MutexGuard, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

static IDS: AtomicU64 = AtomicU64::new(1);
static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
const MAX_INSTANCES: usize = 64;
#[derive(Default)]
struct Registry {
    instances: HashMap<u64, Arc<Instance>>,
    creating: usize,
}
fn registry() -> &'static Mutex<Registry> {
    REGISTRY.get_or_init(Default::default)
}
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}
pub(crate) fn next_id() -> Result<u64, Status> {
    IDS.try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .map_err(|_| RESOURCE_LIMIT)
}

trait Handler: Send + Sync {
    fn handle(&self, request: Request, context: &CallContext<'_>) -> Result<Reply, Failure>;
}
impl<F: Filesystem> Handler for F {
    fn handle(&self, request: Request, context: &CallContext<'_>) -> Result<Reply, Failure> {
        Filesystem::handle(self, request, context)
    }
}
pub(crate) struct Instance {
    pub config: Config,
    owner: TypeId,
    handler: Arc<dyn Handler>,
    state: Mutex<State>,
}
#[derive(Default)]
struct State {
    active: u32,
    reservations: u32,
    bytes: u64,
    buffers: HashMap<u64, Vec<u8>>,
    closing: bool,
    panicked: bool,
    #[cfg(test)]
    fail_publish: bool,
}

struct Creating;
impl Creating {
    fn acquire() -> Result<Self, Status> {
        let mut registry = lock(registry());
        if registry.instances.len() + registry.creating >= MAX_INSTANCES {
            return Err(RESOURCE_LIMIT);
        }
        let additional = registry.creating + 1;
        registry
            .instances
            .try_reserve(additional)
            .map_err(|_| RESOURCE_LIMIT)?;
        registry.creating += 1;
        Ok(Self)
    }
}
impl Drop for Creating {
    fn drop(&mut self) {
        lock(registry()).creating -= 1;
    }
}

pub(crate) fn create<F: Filesystem>(config: Config) -> Result<u64, Status> {
    config.validate()?;
    let creating = Creating::acquire()?;
    let id = next_id()?;
    let handler = F::create(config).map_err(|error| match error.code {
        crate::api::ErrorCode::Unsupported => UNSUPPORTED,
        crate::api::ErrorCode::ResourceLimit => RESOURCE_LIMIT,
        _ => INVALID,
    })?;
    let mut state = State::default();
    state
        .buffers
        .try_reserve(config.max_buffers as usize)
        .map_err(|_| RESOURCE_LIMIT)?;
    let instance = Arc::new(Instance {
        config,
        owner: TypeId::of::<F>(),
        handler: Arc::new(handler),
        state: Mutex::new(state),
    });
    lock(registry()).instances.insert(id, instance);
    drop(creating);
    Ok(id)
}
pub(crate) fn acquire<F: Filesystem>(id: u64) -> Result<Arc<Instance>, Status> {
    lock(registry())
        .instances
        .get(&id)
        .filter(|instance| instance.owner == TypeId::of::<F>())
        .cloned()
        .ok_or(UNKNOWN_HANDLE)
}
pub(crate) struct Permit {
    instance: Arc<Instance>,
    reserved: bool,
}
impl Permit {
    pub fn acquire(instance: Arc<Instance>) -> Result<Self, Status> {
        {
            let mut state = lock(&instance.state);
            if state.closing || state.panicked {
                return Err(SHUTTING_DOWN);
            }
            if state.active >= instance.config.max_calls {
                return Err(BUSY);
            }
            if state.buffers.len() + state.reservations as usize
                >= instance.config.max_buffers as usize
            {
                return Err(RESOURCE_LIMIT);
            }
            let reserved = u64::from(state.reservations + 1)
                .checked_mul(instance.config.reply_bytes)
                .ok_or(RESOURCE_LIMIT)?;
            if state
                .bytes
                .checked_add(reserved)
                .is_none_or(|bytes| bytes > instance.config.retained_buffer_bytes)
            {
                return Err(RESOURCE_LIMIT);
            }
            state.active += 1;
            state.reservations += 1;
        }
        Ok(Self {
            instance,
            reserved: true,
        })
    }
    pub fn run(&self, request: Request, context: &CallContext<'_>) -> Result<Reply, Status> {
        match catch_unwind(AssertUnwindSafe(|| {
            self.instance.handler.handle(request, context)
        })) {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(error)) => Reply::stateless(Response::Failed(error)).map_err(|_| INVALID),
            Err(_) => {
                lock(&self.instance.state).panicked = true;
                Err(PANICKED)
            }
        }
    }
    pub fn completion(&self, completion: Option<crate::reply::Completion>) -> Publication {
        Publication {
            instance: self.instance.clone(),
            completion,
        }
    }
    pub fn publish(&mut self, bytes: Vec<u8>) -> Result<Buffer, Status> {
        #[cfg(test)]
        if std::mem::take(&mut lock(&self.instance.state).fail_publish) {
            return Err(RESOURCE_LIMIT);
        }
        let id = next_id()?;
        let capacity = bytes.capacity() as u64;
        if capacity > self.instance.config.reply_bytes {
            return Err(RESOURCE_LIMIT);
        }
        let buffer = Buffer {
            id,
            bytes: Slice {
                data: bytes.as_ptr(),
                len: bytes.len() as u64,
            },
        };
        let mut state = lock(&self.instance.state);
        state.reservations -= 1;
        self.reserved = false;
        state.bytes = state.bytes.checked_add(capacity).ok_or(RESOURCE_LIMIT)?;
        state.buffers.insert(id, bytes);
        Ok(buffer)
    }
}
#[cfg(test)]
pub(crate) fn fail_next_publish<F: Filesystem>(id: u64) {
    lock(&acquire::<F>(id).unwrap().state).fail_publish = true;
}
pub(crate) struct Publication {
    instance: Arc<Instance>,
    completion: Option<crate::reply::Completion>,
}
impl Publication {
    pub fn finish(&mut self, published: bool) -> Result<(), Status> {
        let Some(completion) = self.completion.take() else {
            return Ok(());
        };
        match catch_unwind(AssertUnwindSafe(|| completion(published))) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) if published && error.code == crate::api::ErrorCode::StaleReference => {
                // The handler completed rollback after its mount/generation retired.
                // No buffer is exposed; other mounts remain usable. A rollback failure
                // (including finish(false)) still poisons admission below.
                Err(UNKNOWN_HANDLE)
            }
            result => {
                lock(&self.instance.state).panicked = true;
                Err(if result.is_err() {
                    PANICKED
                } else {
                    SHUTTING_DOWN
                })
            }
        }
    }
}
impl Drop for Publication {
    fn drop(&mut self) {
        let _ = self.finish(false);
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        let mut state = lock(&self.instance.state);
        state.active -= 1;
        if self.reserved {
            state.reservations -= 1;
        }
    }
}
pub(crate) fn release<F: Filesystem>(instance: u64, id: u64) -> Result<(), Status> {
    let instance = acquire::<F>(instance)?;
    let bytes = {
        let mut state = lock(&instance.state);
        let bytes = state.buffers.remove(&id).ok_or(UNKNOWN_HANDLE)?;
        state.bytes -= bytes.capacity() as u64;
        bytes
    };
    drop(bytes);
    Ok(())
}
pub(crate) fn destroy<F: Filesystem>(id: u64) -> Result<(), Status> {
    let instance = acquire::<F>(id)?;
    {
        let mut state = lock(&instance.state);
        if state.active != 0 || !state.buffers.is_empty() {
            return Err(BUSY);
        }
        state.closing = true;
    }
    let removed = lock(registry())
        .instances
        .remove(&id)
        .ok_or(UNKNOWN_HANDLE)?;
    // The author may close native handles; no registry/state lock covers that cleanup.
    drop(removed);
    drop(instance);
    Ok(())
}
