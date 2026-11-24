use crate::*;
use api::{abi::*, *};
use std::{
    sync::{
        Condvar, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
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
#[test]
fn sdk_never_passes_credentials_to_a_provider_without_sign_in_capability() {
    let table = descriptor::<Fake>();
    let instance = create_instance(&table, Config::default());
    let request = Request::SignIn {
        mount: 1,
        node: 2,
        credentials: NetworkCredentials::new("test".into(), String::new(), b"dummy".to_vec()),
    };
    let (status, buffer) = invoke(&table, instance, request, Cancellation::default());
    assert_eq!(status, UNSUPPORTED);
    assert_eq!(buffer.id, 0);
    assert_eq!(destroy(&table, instance), OK);
}

#[test]
fn sdk_requires_minor_five_when_a_provider_advertises_sign_in() {
    let mut table = Api::default();
    // SAFETY: live aligned caller-owned output table with its exact capacity.
    let status = unsafe {
        entry::<Fake>(
            &mut table,
            size_of::<Api>() as u32,
            "network.test",
            "0.1.0",
            CAP_BROWSE | CAP_ACTIVATE | CAP_SIGN_IN,
        )
    };
    assert_eq!(status, OK);
    assert_eq!(table.header.minimum_host_minor, 5);
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

struct Echo;
impl Filesystem for Echo {
    fn create(_: Config) -> Result<Self, Failure> {
        Ok(Self)
    }
    fn handle(&self, request: Request, _: &CallContext<'_>) -> Result<Reply, Failure> {
        let Request::ReadAt { length, .. } = request else {
            return Reply::stateless(Response::Unit);
        };
        Reply::stateless(Response::Content {
            bytes: vec![7; length as usize],
            eof: false,
        })
    }
}
#[test]
fn retained_replies_are_charged_their_exact_length_not_a_grown_capacity() {
    let api = descriptor::<Echo>();
    // Each 2,119-byte reply is charged exactly, so the third call's 4 KiB reservation
    // fits beside two retained replies. Charging a grown 4 KiB capacity would refuse it.
    let instance = create_instance(
        &api,
        Config {
            reply_bytes: 4096,
            retained_buffer_bytes: 2 * 2119 + 4096,
            ..Config::default()
        },
    );
    let read = Request::ReadAt {
        reader: 1,
        offset: 0,
        length: 2100,
    };
    let mut held = vec![];
    for _ in 0..3 {
        let (status, buffer) = invoke(&api, instance, read.clone(), Cancellation::default());
        assert_eq!(status, OK);
        assert_eq!(buffer.bytes.len, 2119);
        held.push(buffer);
    }
    assert_eq!(
        invoke(&api, instance, read, Cancellation::default()).0,
        RESOURCE_LIMIT
    );
    for buffer in held {
        assert_eq!(release(&api, instance, buffer.id), OK);
    }
    assert_eq!(destroy(&api, instance), OK);
}

/// A diagnostic that embeds a deep native path: 6,002 bytes of two-byte characters.
fn long_diagnostic() -> String {
    format!("x/{}", "é".repeat(3000))
}
struct Verbose;
impl Filesystem for Verbose {
    fn create(_: Config) -> Result<Self, Failure> {
        Ok(Self)
    }
    fn handle(&self, request: Request, _: &CallContext<'_>) -> Result<Reply, Failure> {
        let failure = |code| Failure {
            code,
            os_code: Some(13),
            message: long_diagnostic(),
        };
        match request {
            Request::GetRenameState { prepared } => {
                Reply::stateless(Response::RenameState(RenameState {
                    prepared,
                    attempt: 1,
                    outcome: RenameOutcome::RecoveryRequired {
                        source_location: RenameSourceLocation::Unknown,
                        destination_name: None,
                        recovery: vec![],
                        error: failure(ErrorCode::Io),
                    },
                }))
            }
            Request::VolumeInformation { .. } => {
                Reply::stateless(Response::VolumeInformation(VolumeInformation {
                    unavailable: vec!["a".repeat(MAX_DIAGNOSTIC), long_diagnostic()],
                    ..VolumeInformation::default()
                }))
            }
            _ => Err(failure(ErrorCode::PermissionDenied)),
        }
    }
}
#[test]
fn over_long_author_diagnostics_are_truncated_instead_of_failing_the_call() {
    let api = descriptor::<Verbose>();
    let instance = create_instance(&api, Config::default());
    let truncated = |message: &str| {
        let kept = message.strip_suffix('…').unwrap();
        assert!(message.len() <= MAX_DIAGNOSTIC && message.len() > MAX_DIAGNOSTIC - 8);
        assert!(long_diagnostic().starts_with(kept));
    };
    let address = Address {
        encoding: AddressEncoding::Opaque,
        bytes: b"volume".to_vec(),
    };
    for request in [
        Request::Stat { mount: 1, node: 1 },
        Request::GetRenameState { prepared: 1 },
        Request::VolumeInformation { address },
    ] {
        let (status, buffer) = invoke(&api, instance, request, Cancellation::default());
        assert_eq!(status, OK);
        match read(buffer) {
            Response::Failed(failure) => {
                assert_eq!(failure.code, ErrorCode::PermissionDenied);
                assert_eq!(failure.os_code, Some(13));
                truncated(&failure.message);
            }
            Response::RenameState(RenameState {
                outcome: RenameOutcome::RecoveryRequired { error, .. },
                ..
            }) => {
                assert_eq!((error.code, error.os_code), (ErrorCode::Io, Some(13)));
                truncated(&error.message);
            }
            Response::VolumeInformation(information) => {
                // A diagnostic exactly at the bound is kept unchanged.
                assert_eq!(information.unavailable[0], "a".repeat(MAX_DIAGNOSTIC));
                truncated(&information.unavailable[1]);
            }
            other => panic!("unexpected response {other:?}"),
        }
        assert_eq!(release(&api, instance, buffer.id), OK);
    }
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

/// Holds `Blocking::handle` until the test opens it: (entered, opened).
struct Gate {
    state: Mutex<(bool, bool)>,
    changed: Condvar,
}
impl Gate {
    fn state(&self) -> MutexGuard<'_, (bool, bool)> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
    fn enter(&self) {
        let mut state = self.state();
        state.0 = true;
        self.changed.notify_all();
        while !state.1 {
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
    fn wait_entered(&self) {
        let (state, _) = self
            .changed
            .wait_timeout_while(self.state(), Duration::from_secs(5), |state| !state.0)
            .unwrap_or_else(PoisonError::into_inner);
        assert!(state.0, "the handler did not start");
    }
    fn open(&self) {
        self.state().1 = true;
        self.changed.notify_all();
    }
}
static BLOCKING: Gate = Gate {
    state: Mutex::new((false, false)),
    changed: Condvar::new(),
};
struct Blocking;
impl Filesystem for Blocking {
    fn create(_: Config) -> Result<Self, Failure> {
        Ok(Self)
    }
    fn handle(&self, _: Request, _: &CallContext<'_>) -> Result<Reply, Failure> {
        BLOCKING.enter();
        Reply::stateless(Response::Unit)
    }
}
#[test]
fn active_call_keeps_owner_alive_and_holds_admission_until_actual_return() {
    *BLOCKING.state() = (false, false);
    struct Open;
    impl Drop for Open {
        fn drop(&mut self) {
            // A failed assertion must not leave the worker blocked.
            BLOCKING.open();
        }
    }
    let _open = Open;
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
    BLOCKING.wait_entered();
    assert_eq!(destroy(&api, instance), BUSY);
    assert_eq!(
        invoke(&api, instance, Request::Volumes, Cancellation::default()).0,
        BUSY
    );
    BLOCKING.open();
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
    assert_eq!(table.header.abi_minor, ABI_MINOR);
    let mut forged = table;
    forged.header.minimum_host_minor = 2;
    assert_eq!(forged.validate(size_of::<Api>() as u32), Err(INCOMPATIBLE));
    assert_eq!(size_of::<Api>(), 88);
    table.validate(size_of::<Api>() as u32).unwrap();
}

/// A provider that reports `StorageKind::Local` requires host minor 4 in its descriptor;
/// capability-derived minimums still apply, and a future minor is refused.
#[test]
fn a_required_host_minor_raises_the_descriptor_minimum_only() {
    let fill = |capabilities, minor| {
        let mut table = Api::default();
        // SAFETY: the aligned output is live and covers the declared table capacity.
        let status = unsafe {
            entry_requiring_host_minor::<Fake>(
                &mut table,
                size_of::<Api>() as u32,
                "native.test",
                "0.2.0",
                capabilities,
                minor,
            )
        };
        (status, table)
    };
    let storage = CAP_BROWSE | CAP_STORAGE | CAP_NATIVE_ADDRESS;
    let (status, table) = fill(storage, LOCAL_STORAGE_HOST_MINOR);
    assert_eq!(status, OK);
    assert_eq!(table.header.minimum_host_minor, 4);
    assert_eq!(table.capabilities, storage);
    table.validate(size_of::<Api>() as u32).unwrap();
    let (status, table) = fill(storage | CAP_PREPARED_RENAME, 0);
    assert_eq!(status, OK);
    assert_eq!(table.header.minimum_host_minor, 3);
    let (status, table) = fill(storage | CAP_PREPARED_RENAME, LOCAL_STORAGE_HOST_MINOR);
    assert_eq!(status, OK);
    assert_eq!(table.header.minimum_host_minor, 4);
    assert_eq!(fill(storage, ABI_MINOR + 1).0, INVALID);
}

/// A provider that reports `Local` on every storage question.
struct LocalDisk;
impl Filesystem for LocalDisk {
    fn create(_: Config) -> Result<Self, Failure> {
        Ok(Self)
    }
    fn handle(&self, request: Request, _: &CallContext<'_>) -> Result<Reply, Failure> {
        Reply::stateless(match request {
            Request::StorageKind { .. } => Response::StorageKind(StorageKind::Local),
            Request::VolumeInformation { .. } => Response::VolumeInformation(VolumeInformation {
                storage: StorageKind::Local,
                ..VolumeInformation::default()
            }),
            _ => Response::Unit,
        })
    }
}

/// A host older than minor 4 rejects a reply with storage kind 4, so a descriptor such
/// a host may load answers `Unknown` (that host's answer for the disk) instead; only a
/// descriptor that requires minor 4 sends `Local`.
#[test]
fn local_storage_reaches_only_hosts_that_know_it() {
    let address = Address {
        encoding: AddressEncoding::Opaque,
        bytes: b"volume".to_vec(),
    };
    let storage = CAP_BROWSE | CAP_STORAGE | CAP_NATIVE_ADDRESS;
    for (capabilities, minor, expected) in [
        (storage, 0, StorageKind::Unknown),
        (storage | CAP_PREPARED_RENAME, 0, StorageKind::Unknown),
        (storage, LOCAL_STORAGE_HOST_MINOR, StorageKind::Local),
    ] {
        let mut api = Api::default();
        // SAFETY: the aligned output is live and covers the declared table capacity.
        let status = unsafe {
            entry_requiring_host_minor::<LocalDisk>(
                &mut api,
                size_of::<Api>() as u32,
                "native.test",
                "0.2.0",
                capabilities,
                minor,
            )
        };
        assert_eq!(status, OK);
        let instance = create_instance(&api, Config::default());
        for request in [
            Request::StorageKind {
                address: address.clone(),
            },
            Request::VolumeInformation {
                address: address.clone(),
            },
        ] {
            let (status, buffer) = invoke(&api, instance, request, Cancellation::default());
            assert_eq!(status, OK);
            let reported = match read(buffer) {
                Response::StorageKind(kind) => kind,
                Response::VolumeInformation(information) => information.storage,
                other => panic!("unexpected {other:?}"),
            };
            assert_eq!(reported, expected, "minimum host minor {minor}");
            assert_eq!(release(&api, instance, buffer.id), OK);
        }
        assert_eq!(destroy(&api, instance), OK);
    }
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
