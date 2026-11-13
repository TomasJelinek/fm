use super::*;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
static TESTS: Mutex<()> = Mutex::new(());
static RELEASES: AtomicUsize = AtomicUsize::new(0);
static DESTROYS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn failed_call(_: u64, _: *const abi::Call, out: *mut abi::Buffer) -> u32 {
    // SAFETY: the host owns this output for the exact call duration. This deliberately
    // violates the zero-on-failure contract; it must NOT transfer handle ownership.
    unsafe {
        out.write(abi::Buffer {
            id: 99,
            bytes: abi::Slice::default(),
        });
    }
    abi::BUSY
}
unsafe extern "C" fn release(_: u64, _: u64) -> u32 {
    RELEASES.fetch_add(1, Ordering::Relaxed);
    abi::OK
}
unsafe extern "C" fn destroy(_: u64) -> u32 {
    DESTROYS.fetch_add(1, Ordering::Relaxed);
    abi::OK
}
unsafe extern "C" fn failed_create(_: *const abi::Config, out: *mut u64) -> u32 {
    // SAFETY: exact writable host output; deliberately contradictory failed acquisition.
    unsafe {
        out.write(99);
    }
    abi::INVALID
}

#[test]
fn failed_create_does_not_destroy_an_unowned_sentinel() {
    let _serial = TESTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    DESTROYS.store(0, Ordering::Relaxed);
    assert!(create_instance(failed_create).is_err());
    assert_eq!(DESTROYS.load(Ordering::Relaxed), 0);
}

#[test]
fn failed_destroy_retains_module_until_process_exit() {
    struct Module(std::sync::Arc<AtomicUsize>);
    impl Drop for Module {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    let unloaded = std::sync::Arc::new(AtomicUsize::new(0));
    retire_library(Some(Module(unloaded.clone())), abi::BUSY);
    assert_eq!(
        unloaded.load(Ordering::Relaxed),
        0,
        "Live plugin state must not lose its module"
    );
    retire_library(Some(Module(unloaded.clone())), abi::OK);
    assert_eq!(unloaded.load(Ordering::Relaxed), 1);
}

#[test]
fn failed_call_does_not_adopt_or_release_a_nonzero_output_handle() {
    let _serial = TESTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    RELEASES.store(0, Ordering::Relaxed);
    DESTROYS.store(0, Ordering::Relaxed);
    let instance = Instance {
        library: None,
        instance: 1,
        call: failed_call,
        release,
        destroy,
        poisoned: AtomicBool::new(false),
        admission: crate::admission::Admission::default(),
    };
    assert!(instance.call(&Request::Volumes, &|| false).is_err());
    assert_eq!(
        RELEASES.load(Ordering::Relaxed),
        0,
        "Failure transferred no buffer ownership"
    );
    assert!(
        instance.poisoned.load(Ordering::Acquire),
        "Contradictory failure output must stop this instance"
    );
    drop(instance);
    assert_eq!(
        DESTROYS.load(Ordering::Relaxed),
        1,
        "The successfully created instance remains owned"
    );
}

unsafe extern "C" fn malformed_success(_: u64, _: *const abi::Call, out: *mut abi::Buffer) -> u32 {
    // SAFETY: the static bytes stay mapped until the host releases this owned buffer.
    unsafe {
        out.write(abi::Buffer {
            id: 99,
            bytes: abi::Slice {
                data: b"invalid".as_ptr(),
                len: 7,
            },
        });
    }
    abi::OK
}

#[test]
fn successful_but_malformed_reply_is_released_exactly_once_and_poisoned() {
    let _serial = TESTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    RELEASES.store(0, Ordering::Relaxed);
    let instance = Instance {
        library: None,
        instance: 1,
        call: malformed_success,
        release,
        destroy,
        poisoned: AtomicBool::new(false),
        admission: crate::admission::Admission::default(),
    };
    assert!(instance.call(&Request::Volumes, &|| false).is_err());
    assert_eq!(RELEASES.load(Ordering::Relaxed), 1);
    assert!(instance.poisoned.load(Ordering::Acquire));
    assert!(instance.call(&Request::Volumes, &|| false).is_err());
    assert_eq!(RELEASES.load(Ordering::Relaxed), 1);
}

unsafe extern "C" fn valid_reply(_: u64, _: *const abi::Call, out: *mut abi::Buffer) -> u32 {
    static REPLY: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    let bytes = REPLY.get_or_init(|| {
        midnight_filesystem_plugin_api::encode_response(&Response::Unit, 64).unwrap()
    });
    // SAFETY: the static encoded reply remains mapped for the whole buffer lease.
    unsafe {
        out.write(abi::Buffer {
            id: 99,
            bytes: abi::Slice {
                data: bytes.as_ptr(),
                len: bytes.len() as u64,
            },
        });
    }
    abi::OK
}
unsafe extern "C" fn release_failure(_: u64, _: u64) -> u32 {
    RELEASES.fetch_add(1, Ordering::Relaxed);
    abi::BUSY
}
unsafe extern "C" fn destroy_failure(_: u64) -> u32 {
    DESTROYS.fetch_add(1, Ordering::Relaxed);
    abi::BUSY
}
#[test]
fn release_failure_stops_new_work_without_double_releasing_the_handle() {
    let _serial = TESTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    RELEASES.store(0, Ordering::Relaxed);
    DESTROYS.store(0, Ordering::Relaxed);
    let instance = Instance {
        library: None,
        instance: 1,
        call: valid_reply,
        release: release_failure,
        destroy: destroy_failure,
        poisoned: AtomicBool::new(false),
        admission: crate::admission::Admission::default(),
    };
    assert!(
        instance
            .call(&Request::ValidateRead { reader: 1 }, &|| false)
            .is_err()
    );
    assert!(instance.poisoned.load(Ordering::Acquire));
    assert!(instance.call(&Request::Volumes, &|| false).is_err());
    drop(instance);
    assert_eq!(RELEASES.load(Ordering::Relaxed), 1);
    assert_eq!(DESTROYS.load(Ordering::Relaxed), 1);
}

use midnight_filesystem_plugin_api::{
    DEFAULT_REPLY_BYTES, ErrorCode, Kind, Metadata, NameEncoding, NativeText, PreparedRename,
    RenameObject, RenameOutcome, RenamePolicy, RenameState, encode_response,
};

// The C callbacks exercise the real Instance transport. Every reply is encoded by
// the public API before the call; only the malformed-reply test truncates bytes.
struct RenameFfiReplies {
    replies: Vec<Vec<u8>>,
    next_reply: AtomicUsize,
    calls: Mutex<Vec<(u32, bool)>>,
    releases: Mutex<Vec<u64>>,
    destroys: AtomicUsize,
    call_status: abi::Status,
    release_status: abi::Status,
}
impl RenameFfiReplies {
    fn new(replies: Vec<Response>) -> Self {
        Self {
            replies: replies
                .iter()
                .map(|reply| encode_response(reply, DEFAULT_REPLY_BYTES as usize).unwrap())
                .collect(),
            next_reply: AtomicUsize::new(0),
            calls: Mutex::new(Vec::new()),
            releases: Mutex::new(Vec::new()),
            destroys: AtomicUsize::new(0),
            call_status: abi::OK,
            release_status: abi::OK,
        }
    }

    fn with_instance(&self, check: impl FnOnce(&Instance)) {
        // The shared borrow keeps this callback context at a stable address until
        // the Instance, including its final destroy callback, has been dropped.
        let instance = Instance {
            library: None,
            instance: (self as *const Self) as u64,
            call: rename_ffi_call,
            release: rename_ffi_release,
            destroy: rename_ffi_destroy,
            poisoned: AtomicBool::new(false),
            admission: crate::admission::Admission::default(),
        };
        check(&instance);
        drop(instance);
    }
}
fn rename_ffi_lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

unsafe extern "C" fn rename_ffi_call(
    context: u64,
    input: *const abi::Call,
    output: *mut abi::Buffer,
) -> abi::Status {
    // SAFETY: with_instance retains the fixture; the real Instance::call owns the
    // exact input/output and cancellation context throughout this callback.
    let (fixture, call) = unsafe { (&*(context as *const RenameFfiReplies), &*input) };
    let cancelled = call.cancel.cancelled.is_some_and(|cancelled| {
        // SAFETY: Instance::call retains this documented synchronous callback context.
        unsafe { cancelled(call.cancel.context) != 0 }
    });
    rename_ffi_lock(&fixture.calls).push((call.operation, cancelled));
    if fixture.call_status != abi::OK {
        return fixture.call_status;
    }
    let index = fixture.next_reply.fetch_add(1, Ordering::Relaxed);
    let Some(bytes) = fixture.replies.get(index) else {
        return abi::INVALID;
    };
    // SAFETY: the immutable bytes remain alive through release and final destroy.
    unsafe {
        output.write(abi::Buffer {
            id: index as u64 + 1,
            bytes: abi::Slice {
                data: bytes.as_ptr(),
                len: bytes.len() as u64,
            },
        });
    }
    abi::OK
}
unsafe extern "C" fn rename_ffi_release(context: u64, id: u64) -> abi::Status {
    // SAFETY: with_instance retains the fixture through every release callback.
    let fixture = unsafe { &*(context as *const RenameFfiReplies) };
    rename_ffi_lock(&fixture.releases).push(id);
    fixture.release_status
}
unsafe extern "C" fn rename_ffi_destroy(context: u64) -> abi::Status {
    // SAFETY: with_instance drops the Instance before its borrowed fixture.
    let fixture = unsafe { &*(context as *const RenameFfiReplies) };
    fixture.destroys.fetch_add(1, Ordering::Relaxed);
    abi::OK
}

fn rename_ffi_name(value: &str) -> NativeText {
    NativeText {
        encoding: NameEncoding::Utf8,
        bytes: value.as_bytes().to_vec(),
    }
}
fn rename_ffi_prepared() -> Response {
    Response::RenamePrepared(PreparedRename {
        handle: 7,
        source: RenameObject {
            identity: vec![11],
            revision: vec![12],
            name: rename_ffi_name("before.txt"),
            kind: Kind::File,
            metadata: Metadata {
                size: Some(5),
                ..Metadata::default()
            },
        },
        parent_identity: vec![13],
        recoverable_replace: true,
    })
}
fn rename_ffi_state() -> Response {
    Response::RenameState(RenameState {
        prepared: 7,
        attempt: 1,
        outcome: RenameOutcome::Renamed {
            name: rename_ffi_name("after.txt"),
            same_entry: false,
            recovery: Vec::new(),
            durability_confirmed: false,
        },
    })
}
fn rename_ffi_failed(code: ErrorCode) -> Response {
    Response::Failed(Failure {
        code,
        os_code: Some(5),
        message: "Synthetic provider refusal".into(),
    })
}
fn rename_ffi_request() -> Request {
    Request::Rename {
        prepared: 7,
        attempt: 1,
        name: rename_ffi_name("after.txt"),
        policy: RenamePolicy::NoReplace,
    }
}
fn rename_ffi_pairs() -> [(Request, Response); 4] {
    [
        (
            Request::PrepareRename { mount: 1, node: 2 },
            rename_ffi_prepared(),
        ),
        (rename_ffi_request(), rename_ffi_state()),
        (Request::GetRenameState { prepared: 7 }, rename_ffi_state()),
        (Request::CloseRename { prepared: 7 }, Response::Unit),
    ]
}
fn rename_ffi_accepts(request: &Request, replies: Vec<Response>) {
    for expected in replies {
        let fixture = RenameFfiReplies::new(vec![expected.clone()]);
        fixture.with_instance(|instance| {
            assert_eq!(
                instance.call(request, &|| false),
                Ok(expected),
                "{request:?}"
            );
            assert!(!instance.poisoned.load(Ordering::Acquire), "{request:?}");
            assert_eq!(rename_ffi_lock(&fixture.releases).as_slice(), &[1]);
            assert_eq!(
                rename_ffi_lock(&fixture.calls).as_slice(),
                &[(request.operation() as u32, false)]
            );
        });
        assert_eq!(fixture.destroys.load(Ordering::Relaxed), 1);
    }
}
fn rename_ffi_poison_rejects_controls(instance: &Instance, fixture: &RenameFfiReplies) {
    let before = rename_ffi_lock(&fixture.calls).len();
    for cancelled in [false, true] {
        for request in [
            Request::GetRenameState { prepared: 7 },
            Request::CloseRename { prepared: 7 },
        ] {
            assert!(
                instance.call(&request, &|| cancelled).is_err(),
                "Permanent poison must reject {request:?} with canceled={cancelled}"
            );
        }
    }
    assert_eq!(
        rename_ffi_lock(&fixture.calls).len(),
        before,
        "Cancellation-independent Rename controls cannot bypass permanent poison"
    );
}
fn rename_ffi_rejects(request: &Request, replies: Vec<Response>) {
    for reply in replies {
        let fixture = RenameFfiReplies::new(vec![reply]);
        fixture.with_instance(|instance| {
            let failure = instance.call(request, &|| false).unwrap_err();
            assert_eq!(failure.code, ErrorCode::Invalid, "{request:?}");
            assert!(instance.poisoned.load(Ordering::Acquire), "{request:?}");
            assert_eq!(rename_ffi_lock(&fixture.releases).as_slice(), &[1]);
            assert!(instance.call(&Request::Volumes, &|| false).is_err());
            rename_ffi_poison_rejects_controls(instance, &fixture);
            assert_eq!(
                rename_ffi_lock(&fixture.calls).as_slice(),
                &[(request.operation() as u32, false)],
                "Poison must prevent another callback"
            );
        });
        assert_eq!(fixture.destroys.load(Ordering::Relaxed), 1);
        assert_eq!(rename_ffi_lock(&fixture.releases).as_slice(), &[1]);
    }
}

#[test]
fn rename_foundation_prepare_accepts_prepared_and_failed() {
    let _serial = rename_ffi_lock(&TESTS);
    rename_ffi_accepts(
        &Request::PrepareRename { mount: 1, node: 2 },
        vec![
            rename_ffi_prepared(),
            rename_ffi_failed(ErrorCode::PermissionDenied),
        ],
    );
}

#[test]
fn rename_foundation_rename_accepts_state_and_failed() {
    let _serial = rename_ffi_lock(&TESTS);
    rename_ffi_accepts(
        &rename_ffi_request(),
        vec![
            rename_ffi_state(),
            rename_ffi_failed(ErrorCode::PermissionDenied),
        ],
    );
}

#[test]
fn rename_foundation_query_accepts_state_and_failed() {
    let _serial = rename_ffi_lock(&TESTS);
    rename_ffi_accepts(
        &Request::GetRenameState { prepared: 7 },
        vec![
            rename_ffi_state(),
            rename_ffi_failed(ErrorCode::PermissionDenied),
        ],
    );
}

#[test]
fn rename_foundation_close_accepts_unit() {
    let _serial = rename_ffi_lock(&TESTS);
    rename_ffi_accepts(&Request::CloseRename { prepared: 7 }, vec![Response::Unit]);
}

#[test]
fn rename_foundation_close_busy_preserves_query_and_retry() {
    let _serial = rename_ffi_lock(&TESTS);
    let busy = rename_ffi_failed(ErrorCode::Busy);
    let state = rename_ffi_state();
    let fixture = RenameFfiReplies::new(vec![busy.clone(), state.clone(), Response::Unit]);
    fixture.with_instance(|instance| {
        assert_eq!(
            instance.call(&Request::CloseRename { prepared: 7 }, &|| false),
            Ok(busy)
        );
        assert!(!instance.poisoned.load(Ordering::Acquire));
        assert_eq!(rename_ffi_lock(&fixture.releases).as_slice(), &[1]);
        assert_eq!(
            instance.call(&Request::GetRenameState { prepared: 7 }, &|| false),
            Ok(state)
        );
        assert_eq!(
            instance.call(&Request::CloseRename { prepared: 7 }, &|| false),
            Ok(Response::Unit)
        );
        assert!(!instance.poisoned.load(Ordering::Acquire));
        assert_eq!(rename_ffi_lock(&fixture.releases).as_slice(), &[1, 2, 3]);
        assert_eq!(
            rename_ffi_lock(&fixture.calls).as_slice(),
            &[
                (
                    Request::CloseRename { prepared: 7 }.operation() as u32,
                    false
                ),
                (
                    Request::GetRenameState { prepared: 7 }.operation() as u32,
                    false,
                ),
                (
                    Request::CloseRename { prepared: 7 }.operation() as u32,
                    false
                ),
            ]
        );
    });
    assert_eq!(fixture.destroys.load(Ordering::Relaxed), 1);
}

#[test]
fn rename_foundation_prepare_rejects_wrong_response_classes() {
    let _serial = rename_ffi_lock(&TESTS);
    rename_ffi_rejects(
        &Request::PrepareRename { mount: 1, node: 2 },
        vec![
            Response::Unit,
            rename_ffi_state(),
            Response::Volumes(Vec::new()),
        ],
    );
}

#[test]
fn rename_foundation_rename_rejects_wrong_response_classes() {
    let _serial = rename_ffi_lock(&TESTS);
    rename_ffi_rejects(
        &rename_ffi_request(),
        vec![Response::Unit, Response::Volumes(Vec::new())],
    );
}

#[test]
fn rename_foundation_query_rejects_wrong_response_classes() {
    let _serial = rename_ffi_lock(&TESTS);
    rename_ffi_rejects(
        &Request::GetRenameState { prepared: 7 },
        vec![Response::Unit, Response::Volumes(Vec::new())],
    );
}

#[test]
fn rename_foundation_close_rejects_wrong_response_classes() {
    let _serial = rename_ffi_lock(&TESTS);
    rename_ffi_rejects(
        &Request::CloseRename { prepared: 7 },
        vec![
            rename_ffi_state(),
            Response::Volumes(Vec::new()),
            rename_ffi_failed(ErrorCode::PermissionDenied),
            rename_ffi_failed(ErrorCode::ResourceLimit),
        ],
    );
}

#[test]
fn rename_foundation_close_transport_busy_is_poisoned() {
    let _serial = rename_ffi_lock(&TESTS);
    let mut fixture = RenameFfiReplies::new(Vec::new());
    fixture.call_status = abi::BUSY;
    fixture.with_instance(|instance| {
        let failure = instance
            .call(&Request::CloseRename { prepared: 7 }, &|| false)
            .unwrap_err();
        assert_eq!(failure.code, ErrorCode::Busy);
        assert!(
            instance.poisoned.load(Ordering::Acquire),
            "Only an owned, encoded Failed(Busy) reply permits CloseRename retry"
        );
        assert!(rename_ffi_lock(&fixture.releases).is_empty());
        assert!(instance.call(&Request::Volumes, &|| false).is_err());
        assert_eq!(
            rename_ffi_lock(&fixture.calls).as_slice(),
            &[(
                Request::CloseRename { prepared: 7 }.operation() as u32,
                false
            )]
        );
    });
    assert_eq!(fixture.destroys.load(Ordering::Relaxed), 1);
}

#[test]
fn rename_foundation_legacy_close_keeps_strict_reply_class() {
    let _serial = rename_ffi_lock(&TESTS);
    for request in [
        Request::CloseRead { reader: 3 },
        Request::CloseList { cursor: 4 },
        Request::CloseMount { mount: 1 },
    ] {
        for reply in [rename_ffi_failed(ErrorCode::Busy), rename_ffi_state()] {
            let fixture = RenameFfiReplies::new(vec![reply]);
            fixture.with_instance(|instance| {
                let _response = instance.call(&request, &|| false);
                assert!(
                    instance.poisoned.load(Ordering::Acquire),
                    "Legacy close still requires Unit: {request:?}"
                );
                assert_eq!(rename_ffi_lock(&fixture.releases).as_slice(), &[1]);
                assert!(instance.call(&Request::Volumes, &|| false).is_err());
                assert_eq!(
                    rename_ffi_lock(&fixture.calls).as_slice(),
                    &[(request.operation() as u32, false)]
                );
            });
            assert_eq!(fixture.destroys.load(Ordering::Relaxed), 1);
        }
    }
}

#[test]
fn rename_foundation_malformed_owned_replies_release_once_and_poison() {
    let _serial = rename_ffi_lock(&TESTS);
    for (request, reply) in rename_ffi_pairs() {
        let mut fixture = RenameFfiReplies::new(vec![reply]);
        fixture.replies[0].pop().unwrap();
        fixture.with_instance(|instance| {
            let failure = instance.call(&request, &|| false).unwrap_err();
            assert_eq!(failure.code, ErrorCode::Invalid, "{request:?}");
            assert!(instance.poisoned.load(Ordering::Acquire), "{request:?}");
            assert_eq!(rename_ffi_lock(&fixture.releases).as_slice(), &[1]);
            assert!(instance.call(&Request::Volumes, &|| false).is_err());
            rename_ffi_poison_rejects_controls(instance, &fixture);
            assert_eq!(
                rename_ffi_lock(&fixture.calls).as_slice(),
                &[(request.operation() as u32, false)]
            );
        });
        assert_eq!(fixture.destroys.load(Ordering::Relaxed), 1);
        assert_eq!(rename_ffi_lock(&fixture.releases).as_slice(), &[1]);
    }
}

#[test]
fn rename_foundation_owned_reply_release_failure_poisons_without_double_release() {
    let _serial = rename_ffi_lock(&TESTS);
    for (request, reply) in rename_ffi_pairs() {
        let mut fixture = RenameFfiReplies::new(vec![reply]);
        fixture.release_status = abi::BUSY;
        fixture.with_instance(|instance| {
            let failure = instance.call(&request, &|| false).unwrap_err();
            assert_eq!(failure.code, ErrorCode::Invalid, "{request:?}");
            assert!(instance.poisoned.load(Ordering::Acquire), "{request:?}");
            assert_eq!(rename_ffi_lock(&fixture.releases).as_slice(), &[1]);
            assert!(instance.call(&Request::Volumes, &|| false).is_err());
            rename_ffi_poison_rejects_controls(instance, &fixture);
            assert_eq!(
                rename_ffi_lock(&fixture.calls).as_slice(),
                &[(request.operation() as u32, false)]
            );
        });
        assert_eq!(fixture.destroys.load(Ordering::Relaxed), 1);
        assert_eq!(rename_ffi_lock(&fixture.releases).as_slice(), &[1]);
    }
}

#[test]
fn rename_foundation_cancelled_query_reaches_callback_with_independent_token() {
    let _serial = rename_ffi_lock(&TESTS);
    let state = rename_ffi_state();
    let fixture = RenameFfiReplies::new(vec![state.clone()]);
    fixture.with_instance(|instance| {
        assert_eq!(
            instance.call(&Request::GetRenameState { prepared: 7 }, &|| true),
            Ok(state)
        );
        assert!(!instance.poisoned.load(Ordering::Acquire));
        assert_eq!(rename_ffi_lock(&fixture.releases).as_slice(), &[1]);
        assert_eq!(
            rename_ffi_lock(&fixture.calls).as_slice(),
            &[(
                Request::GetRenameState { prepared: 7 }.operation() as u32,
                false,
            )]
        );
    });
    assert_eq!(fixture.destroys.load(Ordering::Relaxed), 1);
}

#[test]
fn rename_foundation_cancelled_close_reaches_callback_with_independent_token() {
    let _serial = rename_ffi_lock(&TESTS);
    let fixture = RenameFfiReplies::new(vec![Response::Unit]);
    fixture.with_instance(|instance| {
        assert_eq!(
            instance.call(&Request::CloseRename { prepared: 7 }, &|| true),
            Ok(Response::Unit)
        );
        assert!(!instance.poisoned.load(Ordering::Acquire));
        assert_eq!(rename_ffi_lock(&fixture.releases).as_slice(), &[1]);
        assert_eq!(
            rename_ffi_lock(&fixture.calls).as_slice(),
            &[(
                Request::CloseRename { prepared: 7 }.operation() as u32,
                false
            )]
        );
    });
    assert_eq!(fixture.destroys.load(Ordering::Relaxed), 1);
}

#[path = "rename_admission_tests.rs"]
mod rename_admission_tests;

#[test]
fn rename_foundation_descriptor_minor_two_with_minimum_host_three_is_rejected() {
    let mut api = abi::Api {
        header: abi::Header::v1(size_of::<abi::Api>() as u32),
        capabilities: abi::CAP_NATIVE_ADDRESS | abi::CAP_PREPARED_RENAME,
        id: abi::Slice {
            data: b"native.rename-foundation".as_ptr(),
            len: b"native.rename-foundation".len() as u64,
        },
        version: abi::Slice {
            data: b"0.2.0".as_ptr(),
            len: b"0.2.0".len() as u64,
        },
        create: Some(failed_create),
        call: Some(valid_reply),
        release_buffer: Some(release),
        destroy: Some(destroy),
    };
    api.header.minimum_host_minor = 3;
    assert!(validate_descriptor(&api).is_ok());
    api.header.abi_minor = 2;
    let failure = validate_descriptor(&api).unwrap_err();
    assert_eq!(failure.code, ErrorCode::Invalid);
    assert_eq!(failure.message, "Invalid native filesystem ABI (2)");
    api.header.abi_minor = 3;
    assert!(validate_descriptor(&api).is_ok());
}
