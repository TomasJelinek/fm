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
