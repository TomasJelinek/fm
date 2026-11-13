use crate::*;
use api::{abi::*, *};
use std::{
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::{Duration, Instant},
};

struct Fake {
    calls: AtomicUsize,
}

#[test]
fn activation_capability_requires_new_host_but_old_native_descriptors_do_not() {
    let old = descriptor::<Fake>();
    assert_eq!(old.header.minimum_host_minor, 0);
    let mut table = Api::default();
    // SAFETY: live aligned caller-owned output table with its exact capacity.
    let status = unsafe {
        entry::<Fake>(
            &mut table,
            size_of::<Api>() as u32,
            "network.test",
            "0.1.0",
            CAP_BROWSE | CAP_ACTIVATE,
        )
    };
    assert_eq!(status, OK);
    assert_eq!(table.header.minimum_host_minor, 2);
    table.validate(size_of::<Api>() as u32).unwrap();
}
impl Filesystem for Fake {
    fn create(_: Config) -> Result<Self, Failure> {
        Ok(Self {
            calls: AtomicUsize::new(0),
        })
    }
    fn handle(&self, request: Request, context: &CallContext<'_>) -> Result<Reply, Failure> {
        if context.is_cancelled() {
            return Err(Failure::new(ErrorCode::Cancelled, "Canceled"));
        }
        if matches!(request, Request::Volumes) {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Reply::stateless(Response::Volumes(vec![]))
        } else if matches!(request, Request::Stat { .. }) {
            let calls = self.calls.load(Ordering::Relaxed) as u64;
            Reply::stateless(Response::SpaceInfo {
                available: calls,
                total: calls,
            })
        } else {
            Reply::stateless(Response::Unit)
        }
    }
}
pub(crate) fn descriptor<F: Filesystem>() -> Api {
    let mut api = Api::default();
    // SAFETY: aligned, live caller-owned output object covers the declared capacity.
    assert_eq!(
        unsafe {
            entry::<F>(
                &mut api,
                size_of::<Api>() as u32,
                "test.fs",
                "0.1.0",
                CAP_BROWSE | CAP_READ,
            )
        },
        OK
    );
    api.validate(size_of::<Api>() as u32).unwrap();
    api
}
pub(crate) fn create_instance(api: &Api, config: Config) -> u64 {
    let mut id = 0;
    // SAFETY: both borrowed input and writable output live until this synchronous call returns.
    assert_eq!(unsafe { api.create.unwrap()(&config, &mut id) }, OK);
    assert_ne!(id, 0);
    id
}
pub(crate) fn invoke(
    api: &Api,
    instance: u64,
    request: Request,
    cancel: Cancellation,
) -> (Status, Buffer) {
    let bytes = encode_request(&request, DEFAULT_REQUEST_BYTES as usize).unwrap();
    let call = Call {
        struct_size: size_of::<Call>() as u32,
        operation: request.operation() as u32,
        payload: Slice {
            data: bytes.as_ptr(),
            len: bytes.len() as u64,
        },
        cancel,
    };
    let mut output = Buffer::default();
    // SAFETY: the input payload/callback and output are valid for this call's borrow.
    let status = unsafe { api.call.unwrap()(instance, &call, &mut output) };
    (status, output)
}
pub(crate) fn release(api: &Api, instance: u64, buffer: u64) -> Status {
    // SAFETY: trusted descriptor owns this instance; integer ownership is validated by the SDK.
    unsafe { api.release_buffer.unwrap()(instance, buffer) }
}
pub(crate) fn destroy(api: &Api, instance: u64) -> Status {
    // SAFETY: trusted descriptor validates ownership and refuses nonquiescent state.
    unsafe { api.destroy.unwrap()(instance) }
}
pub(crate) fn read(buffer: Buffer) -> Response {
    buffer.validate(DEFAULT_REPLY_BYTES).unwrap();
    // SAFETY: the producing SDK retains this immutable nonempty allocation until release.
    let bytes = unsafe { std::slice::from_raw_parts(buffer.bytes.data, buffer.bytes.len as usize) };
    decode_response(bytes, DEFAULT_REPLY_BYTES as usize).unwrap()
}

#[test]
fn actual_table_roundtrip_retains_reply_until_same_owner_release() {
    let api = descriptor::<Fake>();
    let first = create_instance(&api, Config::default());
    let second = create_instance(&api, Config::default());
    let (status, buffer) = invoke(&api, first, Request::Volumes, Cancellation::default());
    assert_eq!(status, OK);
    assert_eq!(read(buffer), Response::Volumes(vec![]));
    assert_eq!(destroy(&api, first), BUSY);
    assert_eq!(release(&api, second, buffer.id), UNKNOWN_HANDLE);
    assert_eq!(read(buffer), Response::Volumes(vec![]));
    assert_eq!(release(&api, first, buffer.id), OK);
    assert_eq!(release(&api, first, buffer.id), UNKNOWN_HANDLE);
    assert_eq!(destroy(&api, first), OK);
    assert_eq!(destroy(&api, second), OK);
    assert_eq!(destroy(&api, first), UNKNOWN_HANDLE);
}

#[test]
fn buffer_quota_refuses_before_handler_and_resumes_after_release() {
    let api = descriptor::<Fake>();
    let instance = create_instance(
        &api,
        Config {
            max_buffers: 1,
            ..Config::default()
        },
    );
    let (status, buffer) = invoke(&api, instance, Request::Volumes, Cancellation::default());
    assert_eq!(status, OK);
    let (status, rejected) = invoke(&api, instance, Request::Volumes, Cancellation::default());
    assert_eq!(status, RESOURCE_LIMIT);
    assert_eq!(rejected.id, 0);
    assert_eq!(release(&api, instance, buffer.id), OK);
    let (status, counter) = invoke(
        &api,
        instance,
        Request::Stat { mount: 1, node: 1 },
        Cancellation::default(),
    );
    assert_eq!(status, OK);
    assert_eq!(
        read(counter),
        Response::SpaceInfo {
            available: 1,
            total: 1
        },
        "Rejected calls must not advance the author's iterator/state"
    );
    assert_eq!(release(&api, instance, counter.id), OK);
    let (status, buffer) = invoke(&api, instance, Request::Volumes, Cancellation::default());
    assert_eq!(status, OK);
    assert_eq!(release(&api, instance, buffer.id), OK);
    assert_eq!(destroy(&api, instance), OK);
}

#[test]
fn a_foreign_descriptor_cannot_use_another_authors_valid_instance_or_buffer() {
    let first = descriptor::<Fake>();
    let other = descriptor::<Panic>();
    let instance = create_instance(&first, Config::default());
    let (status, buffer) = invoke(&first, instance, Request::Volumes, Cancellation::default());
    assert_eq!(status, OK);
    assert_eq!(release(&other, instance, buffer.id), UNKNOWN_HANDLE);
    assert_eq!(destroy(&other, instance), UNKNOWN_HANDLE);
    assert_eq!(read(buffer), Response::Volumes(vec![]));
    assert_eq!(release(&first, instance, buffer.id), OK);
    assert_eq!(destroy(&first, instance), OK);
}

#[test]
fn short_entry_capacity_and_invalid_config_never_publish_or_overwrite_output() {
    let mut canary = [0x5a5a5a5a5a5a5a5au64; 12];
    let original = canary;
    // SAFETY: storage is aligned and larger than the deliberately short advertised capacity.
    assert_eq!(
        unsafe {
            entry::<Fake>(
                canary.as_mut_ptr().cast(),
                16,
                "test.fs",
                "0.1.0",
                CAP_BROWSE,
            )
        },
        INVALID
    );
    assert_eq!(canary, original);
    let api = descriptor::<Fake>();
    let config = Config {
        reserved: 1,
        ..Config::default()
    };
    let mut instance = 123;
    // SAFETY: both objects are valid and remain borrowed for this synchronous call.
    assert_eq!(
        unsafe { api.create.unwrap()(&config, &mut instance) },
        INVALID
    );
    assert_eq!(instance, 0);
}

unsafe extern "C" fn cancelled(value: u64) -> u32 {
    u32::from(value != 0)
}
#[test]
fn borrowed_cancellation_becomes_a_terminal_failure_without_losing_cleanup() {
    let api = descriptor::<Fake>();
    let instance = create_instance(&api, Config::default());
    let (status, buffer) = invoke(
        &api,
        instance,
        Request::Volumes,
        Cancellation {
            context: 1,
            cancelled: Some(cancelled),
        },
    );
    assert_eq!(status, OK);
    assert!(matches!(
        read(buffer),
        Response::Failed(Failure {
            code: ErrorCode::Cancelled,
            ..
        })
    ));
    assert_eq!(release(&api, instance, buffer.id), OK);
    assert_eq!(destroy(&api, instance), OK);
}

struct Panic;
impl Filesystem for Panic {
    fn create(_: Config) -> Result<Self, Failure> {
        Ok(Self)
    }
    fn handle(&self, _: Request, _: &CallContext<'_>) -> Result<Reply, Failure> {
        panic!("author failure")
    }
}
#[test]
fn author_panic_does_not_unwind_the_abi_and_poisoned_instance_can_be_destroyed() {
    let api = descriptor::<Panic>();
    let instance = create_instance(&api, Config::default());
    let (status, buffer) = invoke(&api, instance, Request::Volumes, Cancellation::default());
    assert_eq!(status, PANICKED);
    assert_eq!(buffer.id, 0);
    assert_eq!(
        invoke(&api, instance, Request::Volumes, Cancellation::default()).0,
        SHUTTING_DOWN
    );
    assert_eq!(destroy(&api, instance), OK);
}

static STARTED: AtomicBool = AtomicBool::new(false);
static FINISH: AtomicBool = AtomicBool::new(false);
struct Blocking;
impl Filesystem for Blocking {
    fn create(_: Config) -> Result<Self, Failure> {
        Ok(Self)
    }
    fn handle(&self, _: Request, _: &CallContext<'_>) -> Result<Reply, Failure> {
        STARTED.store(true, Ordering::Release);
        while !FINISH.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(1));
        }
        Reply::stateless(Response::Unit)
    }
}
#[test]
fn active_call_keeps_owner_alive_and_holds_admission_until_actual_return() {
    STARTED.store(false, Ordering::Release);
    FINISH.store(false, Ordering::Release);
    struct Finish;
    impl Drop for Finish {
        fn drop(&mut self) {
            FINISH.store(true, Ordering::Release);
        }
    }
    let _finish = Finish;
    let api = descriptor::<Blocking>();
    let instance = create_instance(
        &api,
        Config {
            max_calls: 1,
            ..Config::default()
        },
    );
    let call = api.call.unwrap();
    let release = api.release_buffer.unwrap();
    let worker = std::thread::spawn(move || {
        let bytes = encode_request(&Request::Volumes, 1024).unwrap();
        let request = Call {
            struct_size: size_of::<Call>() as u32,
            operation: 12,
            payload: Slice {
                data: bytes.as_ptr(),
                len: bytes.len() as u64,
            },
            cancel: Cancellation::default(),
        };
        let mut output = Buffer::default();
        // SAFETY: all borrowed objects are owned by this thread until return and release.
        let status = unsafe { call(instance, &request, &mut output) };
        if status == OK {
            assert_eq!(unsafe { release(instance, output.id) }, OK);
        }
        status
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !STARTED.load(Ordering::Acquire) {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(destroy(&api, instance), BUSY);
    assert_eq!(
        invoke(&api, instance, Request::Volumes, Cancellation::default()).0,
        BUSY
    );
    FINISH.store(true, Ordering::Release);
    assert_eq!(worker.join().unwrap(), OK);
    assert_eq!(destroy(&api, instance), OK);
}

struct CleanupFailure;
struct RetiredPublication;
impl Filesystem for RetiredPublication {
    fn create(_: Config) -> Result<Self, Failure> {
        Ok(Self)
    }
    fn handle(&self, request: Request, _: &CallContext<'_>) -> Result<Reply, Failure> {
        if matches!(request, Request::Volumes) {
            Ok(Reply::owned(Response::Unit, |published| {
                if published {
                    Err(Failure::new(
                        ErrorCode::StaleReference,
                        "Mount retired; resources rolled back",
                    ))
                } else {
                    Ok(())
                }
            }))
        } else {
            Reply::stateless(Response::Unit)
        }
    }
}
#[test]
fn retired_publication_releases_buffer_without_poisoning_unrelated_requests() {
    let api = descriptor::<RetiredPublication>();
    let instance = create_instance(&api, Config::default());
    let (status, buffer) = invoke(&api, instance, Request::Volumes, Cancellation::default());
    assert_eq!(status, UNKNOWN_HANDLE);
    assert_eq!(buffer.id, 0);
    let (status, buffer) = invoke(
        &api,
        instance,
        Request::CloseMount { mount: 1 },
        Cancellation::default(),
    );
    assert_eq!(status, OK);
    assert_eq!(release(&api, instance, buffer.id), OK);
    assert_eq!(destroy(&api, instance), OK);
}
impl Filesystem for CleanupFailure {
    fn create(_: Config) -> Result<Self, Failure> {
        Ok(Self)
    }
    fn handle(&self, _: Request, _: &CallContext<'_>) -> Result<Reply, Failure> {
        Ok(Reply::owned(Response::Unit, |_| {
            Err(Failure::new(ErrorCode::Io, "cleanup failed"))
        }))
    }
}
#[test]
fn failed_publication_cleanup_poisoning_never_exposes_a_buffer_or_reopens_admission() {
    let api = descriptor::<CleanupFailure>();
    for fail_publish in [false, true] {
        let instance = create_instance(&api, Config::default());
        if fail_publish {
            crate::runtime::fail_next_publish::<CleanupFailure>(instance);
        }
        let (status, buffer) = invoke(&api, instance, Request::Volumes, Cancellation::default());
        assert_eq!(
            status,
            if fail_publish {
                RESOURCE_LIMIT
            } else {
                SHUTTING_DOWN
            }
        );
        assert_eq!(buffer.id, 0);
        assert_eq!(
            invoke(&api, instance, Request::Volumes, Cancellation::default()).0,
            SHUTTING_DOWN
        );
        assert_eq!(destroy(&api, instance), OK);
    }
}

#[test]
fn prepared_rename_capability_requires_minor_three_without_changing_the_table() {
    let mut table = Api::default();
    // SAFETY: the aligned output is live and covers the declared table capacity.
    assert_eq!(
        unsafe {
            entry::<Fake>(
                &mut table,
                size_of::<Api>() as u32,
                "rename.test",
                "0.2.0",
                CAP_BROWSE | 1024,
            )
        },
        OK
    );
    assert_eq!(table.header.minimum_host_minor, 3);
    assert_eq!(table.header.abi_minor, 3);
    let mut forged = table;
    forged.header.minimum_host_minor = 2;
    assert_eq!(forged.validate(size_of::<Api>() as u32), Err(INCOMPATIBLE));
    assert_eq!(size_of::<Api>(), 88);
    table.validate(size_of::<Api>() as u32).unwrap();
}

struct ControlFixture;
impl Filesystem for ControlFixture {
    fn create(_: Config) -> Result<Self, Failure> {
        Ok(Self)
    }
    fn handle(&self, request: Request, context: &CallContext<'_>) -> Result<Reply, Failure> {
        if let Request::ReadAt { .. } = request {
            return Reply::stateless(Response::Content {
                bytes: vec![7; context.config().reply_bytes as usize - 19],
                eof: false,
            });
        }
        if request.operation() as u32 == 22 {
            let mut bytes = b"FSBR\x01\0\0\0\x10\0".to_vec();
            bytes.extend_from_slice(&1u64.to_le_bytes());
            bytes.extend_from_slice(&0u64.to_le_bytes());
            bytes.push(0); // Ready; the raw fixture compiles against the previous API.
            return Reply::stateless(decode_response(&bytes, 1024).unwrap());
        }
        Reply::stateless(Response::Unit)
    }
}
fn invoke_control(api: &Api, instance: u64, operation: u32) -> (Status, Buffer) {
    let mut bytes = b"FSBR\x01\0\0\0".to_vec();
    bytes.extend_from_slice(&1u64.to_le_bytes());
    let call = Call {
        struct_size: size_of::<Call>() as u32,
        operation,
        payload: Slice {
            data: bytes.as_ptr(),
            len: bytes.len() as u64,
        },
        cancel: Cancellation {
            context: 1,
            cancelled: Some(cancelled),
        },
    };
    let mut output = Buffer::default();
    // SAFETY: all descriptor inputs, payload and output remain live for this call.
    let status = unsafe { api.call.unwrap()(instance, &call, &mut output) };
    (status, output)
}
#[test]
fn prepared_rename_control_admission_fits_total_buffer_and_byte_ceilings() {
    for byte_pressure in [false, true] {
        let api = descriptor::<ControlFixture>();
        let config = Config {
            max_calls: 2,
            max_buffers: if byte_pressure { 8 } else { 3 },
            reply_bytes: 4096,
            retained_buffer_bytes: 3 * 4096,
            ..Config::default()
        };
        let instance = create_instance(&api, config);
        let (status, activation) = invoke_control(&api, instance, 22);
        assert_eq!(status, OK);
        assert_eq!(release(&api, instance, activation.id), OK);
        let request = if byte_pressure {
            Request::ReadAt {
                reader: 1,
                offset: 0,
                length: 4077,
            }
        } else {
            Request::Volumes
        };
        let mut held = vec![];
        for _ in 0..2 {
            let (status, buffer) = invoke(&api, instance, request.clone(), Cancellation::default());
            assert_eq!(status, OK);
            if byte_pressure {
                assert_eq!(buffer.bytes.len, 4096);
            }
            held.push(buffer);
        }
        assert_eq!(
            invoke(&api, instance, request, Cancellation::default()).0,
            RESOURCE_LIMIT
        );
        let (status, query) = invoke_control(&api, instance, 22);
        assert_eq!(
            status, OK,
            "ordinary saturation must preserve the reserved share"
        );
        assert!(!matches!(read(query), Response::Failed(_)));
        assert_eq!(
            invoke_control(&api, instance, 23).0,
            RESOURCE_LIMIT,
            "the shared instance ceiling still applies"
        );
        assert_eq!(destroy(&api, instance), BUSY);
        assert_eq!(release(&api, instance, query.id), OK);
        let (status, close) = invoke_control(&api, instance, 23);
        assert_eq!(status, OK);
        assert_eq!(read(close), Response::Unit);
        held.push(close);
        assert!(held.len() <= config.max_buffers as usize);
        assert!(
            held.iter().map(|buffer| buffer.bytes.len).sum::<u64>() <= config.retained_buffer_bytes
        );
        for buffer in held {
            assert_eq!(release(&api, instance, buffer.id), OK);
        }
        assert_eq!(destroy(&api, instance), OK);
    }
}
#[test]
fn prepared_rename_tiny_legacy_configs_keep_browse_but_refuse_rename_admission() {
    for config in [
        Config {
            max_calls: 1,
            ..Config::default()
        },
        Config {
            max_buffers: 1,
            ..Config::default()
        },
        Config {
            retained_buffer_bytes: DEFAULT_REPLY_BYTES,
            ..Config::default()
        },
    ] {
        let api = descriptor::<ControlFixture>();
        let instance = create_instance(&api, config);
        assert_eq!(
            invoke(
                &api,
                instance,
                Request::PrepareRename { mount: 1, node: 2 },
                Cancellation::default()
            )
            .0,
            RESOURCE_LIMIT
        );
        let (status, buffer) = invoke(&api, instance, Request::Volumes, Cancellation::default());
        assert_eq!(status, OK);
        assert_eq!(read(buffer), Response::Unit);
        assert_eq!(release(&api, instance, buffer.id), OK);
        assert_eq!(destroy(&api, instance), OK);
    }
}
