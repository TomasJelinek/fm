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
// First Rename admission reserves one call, one buffer and one maximum reply
// within Config's existing hard ceilings. Legacy-only instances keep all capacity.
const CONTROL_CAPACITY: u32 = 1;
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
struct Lane {
    active: u32,
    reservations: u32,
    bytes: u64,
    buffers: HashMap<u64, Vec<u8>>,
}
#[derive(Default)]
struct State {
    ordinary: Lane,
    control: Lane,
    controls_reserved: bool,
    closing: bool,
    panicked: bool,
    #[cfg(test)]
    fail_publish: bool,
    #[cfg(test)]
    fail_encode: bool,
}
impl State {
    fn lane(&mut self, control: bool) -> &mut Lane {
        if control {
            &mut self.control
        } else {
            &mut self.ordinary
        }
    }
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
        .ordinary
        .buffers
        .try_reserve(config.max_buffers as usize)
        .map_err(|_| RESOURCE_LIMIT)?;
    state
        .control
        .buffers
        .try_reserve(CONTROL_CAPACITY as usize)
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
    control: bool,
}
impl Permit {
    pub fn acquire(instance: Arc<Instance>, operation: u32) -> Result<Self, Status> {
        let control = matches!(operation, 22 | 23);
        {
            let mut state = lock(&instance.state);
            if state.closing || state.panicked {
                return Err(SHUTTING_DOWN);
            }
            let reserve = state.controls_reserved || operation == 20 || control;
            if reserve {
                // Old minima remain valid for Browse/Read. They cannot admit a
                // Rename with independent result retrieval, so refuse before pins.
                if instance.config.max_calls < 2
                    || instance.config.max_buffers < 2
                    || instance.config.retained_buffer_bytes < instance.config.reply_bytes * 2
                {
                    return Err(RESOURCE_LIMIT);
                }
                if state.ordinary.active > instance.config.max_calls - CONTROL_CAPACITY {
                    return Err(BUSY);
                }
                if state.ordinary.buffers.len() + state.ordinary.reservations as usize
                    > (instance.config.max_buffers - CONTROL_CAPACITY) as usize
                {
                    return Err(RESOURCE_LIMIT);
                }
                let used = state.ordinary.bytes
                    + u64::from(state.ordinary.reservations) * instance.config.reply_bytes;
                if used > instance.config.retained_buffer_bytes - instance.config.reply_bytes {
                    return Err(RESOURCE_LIMIT);
                }
            }
            let (calls, buffers, bytes) = if control {
                (
                    CONTROL_CAPACITY,
                    CONTROL_CAPACITY,
                    instance.config.reply_bytes,
                )
            } else if reserve {
                (
                    instance.config.max_calls - CONTROL_CAPACITY,
                    instance.config.max_buffers - CONTROL_CAPACITY,
                    instance.config.retained_buffer_bytes - instance.config.reply_bytes,
                )
            } else {
                (
                    instance.config.max_calls,
                    instance.config.max_buffers,
                    instance.config.retained_buffer_bytes,
                )
            };
            let lane = state.lane(control);
            if lane.active >= calls {
                return Err(BUSY);
            }
            if lane.buffers.len() + lane.reservations as usize >= buffers as usize {
                return Err(RESOURCE_LIMIT);
            }
            let reserved = u64::from(lane.reservations + 1)
                .checked_mul(instance.config.reply_bytes)
                .ok_or(RESOURCE_LIMIT)?;
            if lane
                .bytes
                .checked_add(reserved)
                .is_none_or(|used| used > bytes)
            {
                return Err(RESOURCE_LIMIT);
            }
            lane.active += 1;
            lane.reservations += 1;
            state.controls_reserved = reserve;
        }
        Ok(Self {
            instance,
            reserved: true,
            control,
        })
    }
    pub fn encode(&self, response: &Response) -> Result<Vec<u8>, Status> {
        #[cfg(test)]
        if std::mem::take(&mut lock(&self.instance.state).fail_encode) {
            return Err(RESOURCE_LIMIT);
        }
        crate::api::encode_response(response, self.instance.config.reply_bytes as usize).map_err(
            |error| {
                if error == crate::api::WireError::Limit {
                    RESOURCE_LIMIT
                } else {
                    INVALID
                }
            },
        )
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
        let lane = state.lane(self.control);
        lane.bytes = lane.bytes.checked_add(capacity).ok_or(RESOURCE_LIMIT)?;
        lane.reservations -= 1;
        self.reserved = false;
        lane.buffers.insert(id, bytes);
        Ok(buffer)
    }
}
#[cfg(test)]
pub(crate) fn fail_next_publish<F: Filesystem>(id: u64) {
    lock(&acquire::<F>(id).unwrap().state).fail_publish = true;
}
#[cfg(test)]
pub(crate) fn fail_next_encode<F: Filesystem>(id: u64) {
    lock(&acquire::<F>(id).unwrap().state).fail_encode = true;
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
        let lane = state.lane(self.control);
        lane.active -= 1;
        if self.reserved {
            lane.reservations -= 1;
        }
    }
}
pub(crate) fn release<F: Filesystem>(instance: u64, id: u64) -> Result<(), Status> {
    let instance = acquire::<F>(instance)?;
    let bytes = {
        let mut state = lock(&instance.state);
        if let Some(bytes) = state.ordinary.buffers.remove(&id) {
            state.ordinary.bytes -= bytes.capacity() as u64;
            bytes
        } else {
            let bytes = state.control.buffers.remove(&id).ok_or(UNKNOWN_HANDLE)?;
            state.control.bytes -= bytes.capacity() as u64;
            bytes
        }
    };
    drop(bytes);
    Ok(())
}
pub(crate) fn destroy<F: Filesystem>(id: u64) -> Result<(), Status> {
    let instance = acquire::<F>(id)?;
    {
        let mut state = lock(&instance.state);
        if state.ordinary.active != 0
            || state.control.active != 0
            || !state.ordinary.buffers.is_empty()
            || !state.control.buffers.is_empty()
        {
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
