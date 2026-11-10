use super::*;
use std::{path::PathBuf, sync::atomic::AtomicUsize, time::Instant};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(1);
        let path = std::env::temp_dir().join(format!(
            "midnight-network-gio-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("fixture.txt"), b"local only").unwrap();
        Self(path)
    }
    fn uri(&self) -> Vec<u8> {
        format!("file://{}", self.0.to_str().unwrap()).into_bytes()
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn context(cancelled: &(dyn Fn() -> bool + Sync)) -> Context<'_> {
    Context {
        cancelled,
        deadline: Instant::now() + Duration::from_secs(5),
    }
}

#[test]
fn local_gio_enumerator_completes_and_closes_without_network_access() {
    let directory = Directory::new();
    let mut cursor = Gio.begin(&directory.uri(), &context(&|| false)).unwrap();
    assert!(
        Gio.next(&mut cursor, &context(&|| false))
            .unwrap()
            .is_some()
    );
    assert!(
        Gio.next(&mut cursor, &context(&|| false))
            .unwrap()
            .is_none()
    );
    drop(cursor);
}

#[test]
fn cancellation_after_async_start_drains_callback_before_cursor_reuse() {
    let directory = Directory::new();
    let mut cursor = Gio.begin(&directory.uri(), &context(&|| false)).unwrap();
    let calls = AtomicUsize::new(0);
    let cancel = || calls.fetch_add(1, Ordering::SeqCst) >= 1;
    assert!(matches!(
        Gio.next(&mut cursor, &context(&cancel)),
        Err(Failure {
            code: ErrorCode::Cancelled,
            ..
        })
    ));
    // An outstanding GIO request would return G_IO_ERROR_PENDING here.
    assert!(Gio.next(&mut cursor, &context(&|| false)).is_ok());
    drop(cursor);
}

#[test]
fn cancellation_after_enumeration_start_releases_unpublished_result() {
    let directory = Directory::new();
    let calls = AtomicUsize::new(0);
    let cancel = || calls.fetch_add(1, Ordering::SeqCst) >= 2;
    assert!(matches!(
        Gio.begin(&directory.uri(), &context(&cancel)),
        Err(Failure {
            code: ErrorCode::Cancelled,
            ..
        })
    ));
    let cursor = Gio.begin(&directory.uri(), &context(&|| false)).unwrap();
    drop(cursor);
}

#[test]
#[ignore = "Opt-in real runtime test; requires GIO_USE_VFS=local to prohibit remote discovery"]
fn local_only_runtime_reports_network_backend_unavailable() {
    assert_eq!(std::env::var("GIO_USE_VFS").as_deref(), Ok("local"));
    assert!(matches!(
        Gio.begin(b"network:///", &context(&|| false)),
        Err(Failure {
            code: ErrorCode::Unavailable,
            ..
        })
    ));
}

#[test]
fn native_error_mapping_does_not_expose_diagnostics() {
    let api = runtime().unwrap();
    // SAFETY: fixed GLib ABI; library reference retained by api throughout use.
    let make_error = unsafe {
        *api._libraries[2]
            .get::<unsafe extern "C" fn(u32, c_int, *const c_char) -> *mut GError>(
                b"g_error_new_literal\0",
            )
            .unwrap()
    };
    for (code, expected) in [
        (14, ErrorCode::PermissionDenied),
        (15, ErrorCode::Unavailable),
        (16, ErrorCode::MountRequired),
        (19, ErrorCode::Cancelled),
        (24, ErrorCode::TimedOut),
        (37, ErrorCode::Offline),
        (38, ErrorCode::Offline),
    ] {
        // SAFETY: static valid message, ownership transferred to owned_error.
        let error = unsafe {
            make_error(
                (api.g_io_error_quark)(),
                code,
                c"smb://user:secret@private.example/share".as_ptr(),
            )
        };
        let failure = owned_error(&api, error);
        assert_eq!(failure.code, expected);
        assert!(!failure.message.contains("secret"));
        assert!(!failure.message.contains("private.example"));
    }
}

#[test]
fn unsuccessful_boolean_completion_is_not_reported_as_success() {
    unsafe extern "C" fn failure_without_error(_: Object, _: Object, _: *mut *mut GError) -> c_int {
        0
    }
    let api = runtime().unwrap();
    let main = MainContext::new(&api).unwrap();
    let result = run(
        &api,
        &main,
        &context(&|| false),
        Finish::Boolean(failure_without_error),
        |_, callback, data| {
            // SAFETY: fixture callback ignores the null objects and consumes the live completion.
            unsafe {
                callback(ptr::null_mut(), ptr::null_mut(), data);
            }
        },
    );
    assert!(matches!(
        result,
        Err(Failure {
            code: ErrorCode::Io,
            ..
        })
    ));
}

#[test]
fn password_challenge_is_recorded_and_aborted_without_credentials() {
    use std::sync::atomic::AtomicI32;
    unsafe extern "C" fn reply(operation: Object, value: c_int) {
        // SAFETY: this fixture supplies an AtomicI32 in place of an OS operation.
        unsafe { &*operation.cast::<AtomicI32>() }.store(value, Ordering::SeqCst);
    }
    let replied = AtomicI32::new(-1);
    let mut challenge = Challenge {
        reply,
        password: AtomicBool::new(false),
    };
    let data = (&mut challenge as *mut Challenge).cast();
    let operation = (&replied as *const AtomicI32).cast_mut().cast();
    // SAFETY: handlers do not access message pointers, and fixture state outlives both calls.
    unsafe {
        question_requested(operation, ptr::null(), ptr::null(), data);
    }
    assert_eq!(replied.load(Ordering::SeqCst), 1);
    assert!(!challenge.password.load(Ordering::Acquire));
    replied.store(-1, Ordering::SeqCst);
    // SAFETY: same valid fixture state; no credentials are supplied or retained.
    unsafe {
        password_requested(operation, ptr::null(), ptr::null(), ptr::null(), 0, data);
    }
    assert_eq!(replied.load(Ordering::SeqCst), 1);
    assert!(challenge.password.load(Ordering::Acquire));
}
