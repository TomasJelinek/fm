//! Thread-safe bounded callbacks. The registry uses integer cookies, never native pointers.
use crate::{
    api::{abi, codec, messages, viewer, wire},
    loaded,
    session::{Shared, lock},
};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, SyncSender},
    },
};

pub(crate) struct Job(pub abi::Work);
// SAFETY: accepted ABI Work explicitly transfers its context for host worker execution.
unsafe impl Send for Job {}
pub(crate) enum ServiceKind {
    Read(messages::ReadAt),
    Close(messages::CloseReader),
}
pub(crate) struct Service {
    pub id: u64,
    pub kind: ServiceKind,
}
struct HostBuffer {
    bytes: Box<[u8]>,
    references: usize,
}
#[derive(Default)]
struct Buffers {
    values: HashMap<u64, HostBuffer>,
    bytes: usize,
}

pub(crate) struct Bridge {
    pub shared: Arc<Shared>,
    cpu: SyncSender<Job>,
    io: SyncSender<Job>,
    service: SyncSender<Service>,
    pub jobs: AtomicUsize,
    pub services: AtomicUsize,
    pub stop: AtomicBool,
    buffers: Mutex<Buffers>,
    ids: AtomicU64,
}
static IDS: AtomicU64 = AtomicU64::new(1);
static REGISTRY: OnceLock<Mutex<HashMap<u64, Weak<Bridge>>>> = OnceLock::new();
fn registry() -> &'static Mutex<HashMap<u64, Weak<Bridge>>> {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}
fn context(id: u64) -> Option<Arc<Bridge>> {
    lock(registry()).get(&id).and_then(Weak::upgrade)
}
fn boundary(work: impl FnOnce() -> abi::Status) -> abi::Status {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)) {
        Ok(status) => status,
        Err(payload) => {
            std::mem::forget(payload);
            abi::PANICKED
        }
    }
}
pub(crate) struct Registration(pub u64);
impl Drop for Registration {
    fn drop(&mut self) {
        lock(registry()).remove(&self.0);
    }
}
pub(crate) struct Receivers {
    pub cpu: mpsc::Receiver<Job>,
    pub io: mpsc::Receiver<Job>,
    pub service: mpsc::Receiver<Service>,
}

impl Bridge {
    pub fn new(shared: Arc<Shared>) -> (Arc<Self>, Receivers) {
        let (cpu, cpu_rx) = mpsc::sync_channel(32);
        let (io, io_rx) = mpsc::sync_channel(32);
        let (service, service_rx) = mpsc::sync_channel(32);
        (
            Arc::new(Self {
                shared,
                cpu,
                io,
                service,
                jobs: AtomicUsize::new(0),
                services: AtomicUsize::new(0),
                stop: AtomicBool::new(false),
                buffers: Mutex::new(Buffers::default()),
                ids: AtomicU64::new(1),
            }),
            Receivers {
                cpu: cpu_rx,
                io: io_rx,
                service: service_rx,
            },
        )
    }
    pub fn register(self: &Arc<Self>) -> Result<(Registration, abi::HostApi), crate::Error> {
        let id = IDS
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| crate::Error("Host context IDs exhausted".into()))?;
        lock(registry()).insert(id, Arc::downgrade(self));
        Ok((
            Registration(id),
            abi::HostApi {
                header: abi::Header {
                    struct_size: size_of::<abi::HostApi>() as u32,
                    abi_major: abi::ABI_MAJOR,
                    abi_minor: abi::ABI_MINOR,
                    kind: abi::KIND_HOST,
                    minimum_host_minor: 0,
                    reserved: 0,
                },
                context: id,
                schedule_work: Some(schedule),
                cancelled: Some(cancelled),
                submit_service: Some(service),
                notify_ready: Some(notify),
                retain_buffer: Some(retain),
                release_buffer: Some(release),
                diagnostic: Some(diagnostic),
            },
        ))
    }
    pub fn allocate(&self, bytes: Vec<u8>) -> Result<abi::Buffer, abi::Status> {
        if bytes.is_empty() {
            return Ok(loaded::empty());
        }
        if bytes.len() > viewer::MAX_PAGE_BYTES {
            return Err(abi::INVALID);
        }
        let mut buffers = lock(&self.buffers);
        if buffers.values.len() >= 32 || bytes.len() > 4 * 1024 * 1024 - buffers.bytes {
            return Err(abi::QUEUE_FULL);
        }
        let id = self
            .ids
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| abi::INVALID)?;
        let bytes = bytes.into_boxed_slice();
        let buffer = abi::Buffer {
            id,
            bytes: loaded::slice(&bytes),
        };
        buffers.bytes += bytes.len();
        buffers.values.insert(
            id,
            HostBuffer {
                bytes,
                references: 1,
            },
        );
        Ok(buffer)
    }
    pub fn release(&self, id: u64) -> abi::Status {
        if id == 0 {
            return abi::OK;
        }
        let mut buffers = lock(&self.buffers);
        let Some(buffer) = buffers.values.get_mut(&id) else {
            return abi::UNKNOWN_HANDLE;
        };
        buffer.references -= 1;
        if buffer.references == 0 {
            let bytes = buffer.bytes.len();
            buffers.values.remove(&id);
            buffers.bytes -= bytes;
        }
        abi::OK
    }
    pub fn empty(&self) -> bool {
        self.jobs.load(Ordering::Acquire) == 0
            && self.services.load(Ordering::Acquire) == 0
            && lock(&self.buffers).values.is_empty()
    }
}

unsafe extern "C" fn schedule(id: u64, work: *const abi::Work) -> abi::Status {
    boundary(|| {
        let Some(bridge) = context(id) else {
            return abi::UNKNOWN_HANDLE;
        };
        if work.is_null() || !work.is_aligned() {
            return abi::INVALID;
        }
        // SAFETY: ABI caller borrows one mapped complete Work for this call.
        let work = unsafe { work.read() };
        if work.run.is_none() || work.drop.is_none() || work.request_id == 0 {
            return abi::INVALID;
        }
        if work.affinity != 0 {
            return abi::UNSUPPORTED;
        }
        let sender = match work.class {
            abi::WORK_CPU => &bridge.cpu,
            abi::WORK_IO => &bridge.io,
            _ => return abi::INVALID,
        };
        if bridge.shared.closing.load(Ordering::Acquire) {
            return abi::SHUTTING_DOWN;
        }
        bridge.jobs.fetch_add(1, Ordering::Acquire);
        if sender.try_send(Job(work)).is_err() {
            bridge.jobs.fetch_sub(1, Ordering::Release);
            return abi::QUEUE_FULL;
        }
        abi::OK
    })
}
unsafe extern "C" fn service(id: u64, request: *const abi::Request) -> abi::Status {
    boundary(|| {
        let Some(bridge) = context(id) else {
            return abi::UNKNOWN_HANDLE;
        };
        if request.is_null() || !request.is_aligned() {
            return abi::INVALID;
        }
        // SAFETY: caller borrows a complete Request with mapped payload during this call.
        let request = unsafe { request.read() };
        if request.id == 0
            || request.session != 1
            || request.generation != bridge.shared.generation
            || request.flags != 0
            || request.content.id != 0
            || request.content.bytes.len != 0
        {
            return abi::INVALID;
        }
        if bridge.shared.closing.load(Ordering::Acquire) {
            return abi::SHUTTING_DOWN;
        }
        let Ok(payload) =
            (unsafe { loaded::bytes(request.payload, wire::MAX_MESSAGE_BYTES as u32) })
        else {
            return abi::INVALID;
        };
        let kind = match request.kind {
            abi::SERVICE_READ_AT => match codec::decode(payload) {
                Ok(read) => ServiceKind::Read(read),
                Err(_) => return abi::INVALID,
            },
            abi::SERVICE_CLOSE_READER => match codec::decode(payload) {
                Ok(close) => ServiceKind::Close(close),
                Err(_) => return abi::INVALID,
            },
            _ => return abi::UNSUPPORTED,
        };
        bridge.services.fetch_add(1, Ordering::Acquire);
        if bridge
            .service
            .try_send(Service {
                id: request.id,
                kind,
            })
            .is_err()
        {
            bridge.services.fetch_sub(1, Ordering::Release);
            return abi::QUEUE_FULL;
        }
        abi::OK
    })
}
unsafe extern "C" fn cancelled(id: u64, request: u64) -> u32 {
    u32::from(context(id).is_none_or(|bridge| bridge.shared.cancelled(request)))
}
unsafe extern "C" fn notify(_: u64) { /* Dispatcher has a bounded 2 ms poll; never reenter plugin/UI. */
}
unsafe extern "C" fn retain(id: u64, buffer: u64) -> abi::Status {
    boundary(|| {
        let Some(bridge) = context(id) else {
            return abi::UNKNOWN_HANDLE;
        };
        let mut buffers = lock(&bridge.buffers);
        let Some(buffer) = buffers.values.get_mut(&buffer) else {
            return abi::UNKNOWN_HANDLE;
        };
        if buffer.references >= 1024 {
            return abi::QUEUE_FULL;
        }
        buffer.references += 1;
        abi::OK
    })
}
unsafe extern "C" fn release(id: u64, buffer: u64) -> abi::Status {
    boundary(|| context(id).map_or(abi::UNKNOWN_HANDLE, |bridge| bridge.release(buffer)))
}
unsafe extern "C" fn diagnostic(_: u64, _: u32, _: abi::ByteSlice) { /* Native diagnostic content is intentionally not logged. */
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::tests::shared;
    unsafe extern "C" fn noop(_: *mut std::ffi::c_void) {}

    #[test]
    fn workers_run_and_drop_accepted_jobs_once_and_drop_cancelled_jobs_without_running() {
        struct Counts {
            run: AtomicUsize,
            drop: AtomicUsize,
        }
        unsafe extern "C" fn run(context: *mut std::ffi::c_void) {
            // SAFETY: fixture keeps its shared counters live until the worker joins.
            unsafe { &*context.cast::<Counts>() }
                .run
                .fetch_add(1, Ordering::Relaxed);
        }
        unsafe extern "C" fn drop_job(context: *mut std::ffi::c_void) {
            // SAFETY: same shared counters; callback consumes one job's ownership ticket.
            unsafe { &*context.cast::<Counts>() }
                .drop
                .fetch_add(1, Ordering::Relaxed);
        }
        for cancel in [false, true] {
            let counts = Counts {
                run: AtomicUsize::new(0),
                drop: AtomicUsize::new(0),
            };
            let shared = shared();
            lock(&shared.state).requests.insert(1, false);
            let (bridge, receivers) = Bridge::new(shared.clone());
            let (_registration, api) = bridge.register().unwrap();
            let work = abi::Work {
                context: (&counts as *const Counts).cast_mut().cast(),
                run: Some(run),
                drop: Some(drop_job),
                request_id: 1,
                class: abi::WORK_CPU,
                affinity: 0,
            };
            for _ in 0..32 {
                assert_eq!(
                    unsafe { api.schedule_work.unwrap()(api.context, &work) },
                    abi::OK
                );
            }
            assert_eq!(
                unsafe { api.schedule_work.unwrap()(api.context, &work) },
                abi::QUEUE_FULL
            );
            assert_eq!(
                counts.drop.load(Ordering::Acquire),
                0,
                "rejected scheduling consumed ownership"
            );
            shared.closing.store(cancel, Ordering::Release);
            let worker_bridge = bridge.clone();
            let worker =
                std::thread::spawn(move || crate::session::run_jobs(worker_bridge, receivers.cpu));
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while bridge.jobs.load(Ordering::Acquire) != 0 && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            bridge.stop.store(true, Ordering::Release);
            worker.join().unwrap();
            assert_eq!(
                counts.run.load(Ordering::Acquire),
                if cancel { 0 } else { 32 }
            );
            assert_eq!(counts.drop.load(Ordering::Acquire), 32);
            assert!(bridge.empty());
        }
    }

    #[test]
    fn work_queue_rejection_preserves_ownership_and_shutdown_has_reserved_capacity() {
        let shared = shared();
        lock(&shared.state).requests.insert(1, false);
        let (bridge, receivers) = Bridge::new(shared.clone());
        let (_registration, api) = bridge.register().unwrap();
        let work = abi::Work {
            context: std::ptr::null_mut(),
            run: Some(noop),
            drop: Some(noop),
            request_id: 1,
            class: abi::WORK_CPU,
            affinity: 0,
        };
        for _ in 0..32 {
            assert_eq!(
                unsafe { api.schedule_work.unwrap()(api.context, &work) },
                abi::OK
            );
        }
        assert_eq!(
            unsafe { api.schedule_work.unwrap()(api.context, &work) },
            abi::QUEUE_FULL
        );
        assert_eq!(bridge.jobs.load(Ordering::Acquire), 32);
        shared.closing.store(true, Ordering::Release);
        assert_eq!(unsafe { api.cancelled.unwrap()(api.context, 1) }, 1);
        assert_eq!(
            unsafe { api.schedule_work.unwrap()(api.context, &work) },
            abi::SHUTTING_DOWN
        );
        for _ in 0..32 {
            let job = receivers.cpu.try_recv().unwrap();
            unsafe {
                job.0.drop.unwrap()(job.0.context);
            }
            bridge.jobs.fetch_sub(1, Ordering::Release);
        }
        assert!(bridge.empty());
    }

    #[test]
    fn retained_host_buffers_are_bounded_scoped_and_released_exactly_once() {
        let (bridge, _) = Bridge::new(shared());
        let (registration, api) = bridge.register().unwrap();
        let buffer = bridge.allocate(vec![0, 128, 255]).unwrap();
        assert_eq!(
            unsafe { api.retain_buffer.unwrap()(api.context, buffer.id) },
            abi::OK
        );
        assert_eq!(bridge.release(buffer.id), abi::OK);
        assert!(!bridge.empty());
        assert_eq!(
            unsafe { loaded::bytes(buffer.bytes, 3).unwrap() },
            &[0, 128, 255]
        );
        assert_eq!(bridge.release(buffer.id), abi::OK);
        assert_eq!(bridge.release(buffer.id), abi::UNKNOWN_HANDLE);
        assert!(bridge.empty());
        let mut buffers = Vec::new();
        for _ in 0..16 {
            buffers.push(bridge.allocate(vec![0; viewer::MAX_PAGE_BYTES]).unwrap());
        }
        assert_eq!(bridge.allocate(vec![0]).unwrap_err(), abi::QUEUE_FULL);
        for buffer in buffers {
            assert_eq!(bridge.release(buffer.id), abi::OK);
        }
        assert!(bridge.empty());
        drop(registration);
        assert_eq!(
            unsafe { api.retain_buffer.unwrap()(api.context, 1) },
            abi::UNKNOWN_HANDLE
        );
        assert_eq!(unsafe { api.cancelled.unwrap()(api.context, 1) }, 1);
    }

    #[test]
    fn service_requests_are_bounded_and_validate_reader_scope_and_ranges() {
        let (bridge, _receivers) = Bridge::new(shared());
        let (_registration, api) = bridge.register().unwrap();
        let payload = codec::encode(&messages::ReadAt {
            reader: 1,
            revision: 4,
            offset: 1 << 34,
            length: 4,
        })
        .unwrap();
        let mut request = abi::Request {
            id: 1,
            session: 1,
            generation: 7,
            kind: abi::SERVICE_READ_AT,
            flags: 0,
            payload: loaded::slice(&payload),
            content: loaded::empty(),
        };
        request.generation = 8;
        assert_eq!(
            unsafe { api.submit_service.unwrap()(api.context, &request) },
            abi::INVALID
        );
        request.generation = 7;
        for _ in 0..32 {
            assert_eq!(
                unsafe { api.submit_service.unwrap()(api.context, &request) },
                abi::OK
            );
        }
        assert_eq!(
            unsafe { api.submit_service.unwrap()(api.context, &request) },
            abi::QUEUE_FULL
        );
    }
}
