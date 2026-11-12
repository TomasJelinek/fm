use super::*;
use std::{
    collections::HashMap,
    sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

pub(crate) struct State {
    bytes: Box<[u8]>,
    retains: AtomicUsize,
    releases: AtomicUsize,
    reject: AtomicBool,
    pub(crate) accept_work: AtomicBool,
    pub(crate) accept_service: AtomicBool,
    work: Mutex<Vec<StoredWork>>,
    pub(crate) services: Mutex<Vec<(u64, Vec<u8>)>>,
}

struct StoredWork {
    context: usize,
    run: unsafe extern "C" fn(*mut std::ffi::c_void),
    drop: unsafe extern "C" fn(*mut std::ffi::c_void),
}

impl State {
    pub(crate) fn run_one(&self) -> bool {
        let work = lock(&self.work).pop();
        if let Some(work) = work {
            // SAFETY: schedule captured a valid SDK job; this fixture owns one run/drop.
            unsafe {
                (work.run)(work.context as *mut _);
                (work.drop)(work.context as *mut _);
            }
            true
        } else {
            false
        }
    }
}

static STATES: OnceLock<Mutex<HashMap<u64, Arc<State>>>> = OnceLock::new();
static IDS: AtomicU64 = AtomicU64::new(1);

fn state(id: u64) -> Arc<State> {
    lock(STATES.get().unwrap()).get(&id).unwrap().clone()
}

extern "C" fn retain(context: u64, id: u64) -> abi::Status {
    let state = state(context);
    if id != 7 || state.reject.load(Ordering::Relaxed) {
        return abi::UNKNOWN_HANDLE;
    }
    state.retains.fetch_add(1, Ordering::Relaxed);
    abi::OK
}
extern "C" fn release(context: u64, id: u64) -> abi::Status {
    assert_eq!(id, 7);
    state(context).releases.fetch_add(1, Ordering::Relaxed);
    abi::OK
}
extern "C" fn cancelled(_: u64, _: u64) -> u32 {
    0
}
extern "C" fn notify(_: u64) {}
extern "C" fn diagnostic(_: u64, _: u32, _: abi::ByteSlice) {}
unsafe extern "C" fn service(context: u64, request: *const abi::Request) -> abi::Status {
    let state = state(context);
    if !state.accept_service.load(Ordering::Relaxed) {
        return abi::UNSUPPORTED;
    }
    // SAFETY: SDK borrows this request and its payload through this callback.
    let request = unsafe { &*request };
    let payload =
        unsafe { std::slice::from_raw_parts(request.payload.data, request.payload.len as usize) };
    lock(&state.services).push((request.id, payload.to_vec()));
    abi::OK
}
unsafe extern "C" fn schedule(context: u64, work: *const abi::Work) -> abi::Status {
    let state = state(context);
    if !state.accept_work.load(Ordering::Relaxed) {
        return abi::QUEUE_FULL;
    }
    // SAFETY: SDK borrows its complete descriptor through this callback.
    let work = unsafe { &*work };
    lock(&state.work).push(StoredWork {
        context: work.context as usize,
        run: work.run.unwrap(),
        drop: work.drop.unwrap(),
    });
    abi::OK
}

pub(crate) fn fixture() -> (Host, Arc<State>, abi::Buffer) {
    let state = Arc::new(State {
        bytes: vec![0, 128, 255, 4].into_boxed_slice(),
        retains: AtomicUsize::new(0),
        releases: AtomicUsize::new(0),
        reject: AtomicBool::new(false),
        accept_work: AtomicBool::new(false),
        accept_service: AtomicBool::new(false),
        work: Mutex::new(Vec::new()),
        services: Mutex::new(Vec::new()),
    });
    let context = IDS.fetch_add(1, Ordering::Relaxed);
    lock(STATES.get_or_init(|| Mutex::new(HashMap::new()))).insert(context, state.clone());
    let api = abi::HostApi {
        header: abi::Header {
            struct_size: size_of::<abi::HostApi>() as u32,
            abi_major: 1,
            abi_minor: 0,
            kind: abi::KIND_HOST,
            minimum_host_minor: 0,
            reserved: 0,
        },
        context,
        schedule_work: Some(schedule),
        cancelled: Some(cancelled),
        submit_service: Some(service),
        notify_ready: Some(notify),
        retain_buffer: Some(retain),
        release_buffer: Some(release),
        diagnostic: Some(diagnostic),
    };
    // All callbacks and their state outlive this test; bytes stay immutable in State.
    let host = unsafe { Host::new(api) }.unwrap();
    let buffer = abi::Buffer {
        id: 7,
        bytes: abi::ByteSlice {
            data: state.bytes.as_ptr(),
            len: 4,
            reserved: 0,
        },
    };
    (host, state, buffer)
}

#[test]
fn retained_input_survives_dispatch_and_releases_once_after_last_worker_copy() {
    let (host, state, buffer) = fixture();
    let content = unsafe { host.retain(buffer) }.unwrap();
    assert_eq!(content.as_ref(), &[0, 128, 255, 4]);
    assert_eq!(host.leases(), 1);
    let worker_copy = content.clone();
    drop(content);
    assert_eq!(state.releases.load(Ordering::Relaxed), 0);
    std::thread::spawn(move || {
        assert_eq!(worker_copy.as_ref(), &[0, 128, 255, 4]);
        drop(worker_copy);
    })
    .join()
    .unwrap();
    assert_eq!(state.retains.load(Ordering::Relaxed), 1);
    assert_eq!(state.releases.load(Ordering::Relaxed), 1);
    assert_eq!(host.leases(), 0);
}

#[test]
fn rejected_and_invalid_buffers_do_not_retain_or_release_host_memory() {
    let (host, state, mut buffer) = fixture();
    state.reject.store(true, Ordering::Relaxed);
    assert_eq!(
        unsafe { host.retain(buffer) }.err(),
        Some(abi::UNKNOWN_HANDLE)
    );
    state.reject.store(false, Ordering::Relaxed);
    buffer.bytes.data = std::ptr::null();
    assert_eq!(unsafe { host.retain(buffer) }.err(), Some(abi::INVALID));
    buffer.bytes.len = 0;
    buffer.id = 0;
    let empty = unsafe { host.retain(buffer) }.unwrap();
    assert!(empty.as_ref().is_empty());
    drop(empty);
    assert_eq!(state.retains.load(Ordering::Relaxed), 0);
    assert_eq!(state.releases.load(Ordering::Relaxed), 0);
    assert_eq!(host.leases(), 0);
}

#[test]
fn retained_allocations_are_bounded_and_capacity_returns_after_release() {
    let (host, state, buffer) = fixture();
    let held: Vec<_> = (0..128)
        .map(|_| unsafe { host.retain(buffer) }.unwrap())
        .collect();
    assert_eq!(unsafe { host.retain(buffer) }.err(), Some(abi::QUEUE_FULL));
    assert_eq!(state.retains.load(Ordering::Relaxed), 128);
    drop(held);
    assert_eq!(host.leases(), 0);
    assert!(unsafe { host.retain(buffer) }.is_ok());
    assert_eq!(state.releases.load(Ordering::Relaxed), 129);
}
