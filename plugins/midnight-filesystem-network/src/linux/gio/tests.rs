use super::*;
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering},
    time::Instant,
};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(1);
        // Fixtures stay in the repository's _build output, never in system temp.
        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../_build/goal/network-plugin/gio-fixtures");
        std::fs::create_dir_all(&base).unwrap();
        let base = base.canonicalize().unwrap();
        assert!(base.components().any(|part| part.as_os_str() == "_build"));
        let path = base.join(format!(
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
fn records_beyond_one_request_batch_are_each_returned_once() {
    let directory = Directory::new();
    for number in 0..(BATCH as usize + 9) {
        std::fs::create_dir(directory.0.join(format!("share-{number:02}"))).unwrap();
    }
    let mut cursor = Gio.begin(&directory.uri(), &context(&|| false)).unwrap();
    let mut displays = Vec::new();
    let mut records = 0;
    while let Some(entry) = Gio.next(&mut cursor, &context(&|| false)).unwrap() {
        records += 1;
        if !entry.display.is_empty() {
            displays.push(entry.display);
        }
    }
    // Directories carry their caption; the regular fixture file is a placeholder row.
    assert_eq!(records, BATCH as usize + 10);
    displays.sort();
    let expected: Vec<_> = (0..(BATCH as usize + 9))
        .map(|number| format!("share-{number:02}"))
        .collect();
    assert_eq!(displays, expected);
    drop(cursor);
}

#[test]
fn a_partly_read_batch_is_released_with_its_cursor() {
    let directory = Directory::new();
    for number in 0..3 {
        std::fs::create_dir(directory.0.join(format!("share-{number}"))).unwrap();
    }
    let mut cursor = Gio.begin(&directory.uri(), &context(&|| false)).unwrap();
    assert!(
        Gio.next(&mut cursor, &context(&|| false))
            .unwrap()
            .is_some()
    );
    assert!(cursor.batch.is_some() && !cursor.unread.is_null());
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

fn ask_password(api: &Symbols, operation: Object, flags: u32) {
    // SAFETY: the operation is live and the arguments match the GLib ask-password signal.
    unsafe {
        let emit = *api._libraries[1]
            .get::<unsafe extern "C" fn(Object, *const c_char, ...)>(b"g_signal_emit_by_name\0")
            .unwrap();
        emit(
            operation,
            c"ask-password".as_ptr(),
            c"Fixture".as_ptr(),
            c"untrusted-user".as_ptr(),
            c"untrusted-domain".as_ptr(),
            flags,
        );
    }
}

#[test]
fn activation_without_credentials_reports_the_password_challenge() {
    let api = runtime().unwrap();
    let context = context(&|| false);
    let mount = MountChallenge::new(&api, &context, None).unwrap();
    ask_password(&api, mount.pointer(), 7);
    assert_eq!(
        mount.check().unwrap_err().code,
        ErrorCode::AuthenticationRequired
    );
}

#[test]
fn canceled_and_unsupported_challenges_leave_password_unset() {
    let api = runtime().unwrap();
    let credentials = NetworkCredentials::new("test".into(), String::new(), b"fixture".to_vec());
    for (cancel, flags, expected) in [
        (true, 7, ErrorCode::Cancelled),
        (false, 33, ErrorCode::Unsupported), // GIO wants additional TCRYPT fields
    ] {
        let canceled = AtomicBool::new(false);
        let predicate = || canceled.load(Ordering::Acquire);
        let context = context(&predicate);
        let mount = MountChallenge::new(&api, &context, Some(&credentials)).unwrap();
        canceled.store(cancel, Ordering::Release);
        ask_password(&api, mount.pointer(), flags);
        // SAFETY: the documented getter reads this still-live GMountOperation.
        unsafe {
            let password = *api._libraries[0]
                .get::<unsafe extern "C" fn(Object) -> *const c_char>(
                    b"g_mount_operation_get_password\0",
                )
                .unwrap();
            assert!(password(mount.pointer()).is_null());
        }
        assert_eq!(mount.check().unwrap_err().code, expected);
    }
}

#[test]
fn explicit_sign_in_answers_only_one_supported_password_prompt() {
    let api = runtime().unwrap();
    let context = context(&|| false);
    let credentials = crate::api::NetworkCredentials::new(
        "uživatel".into(),
        "WORKGROUP".into(),
        b"fixture-password".to_vec(),
    );
    unsafe extern "C" fn record_reply(_: Object, result: c_int, data: Object) {
        // SAFETY: the reply fixture owns this AtomicI32 until the operation is dropped.
        unsafe { &*data.cast::<AtomicI32>() }.store(result, Ordering::Release);
    }
    let reply = AtomicI32::new(-1);
    let mount = MountChallenge::new(&api, &context, Some(&credentials)).unwrap();
    let operation = mount.pointer();
    // SAFETY: these are documented GObject/GIO operations from the retained runtime.
    unsafe {
        let handler = (api.g_signal_connect_data)(
            operation,
            c"reply".as_ptr(),
            std::mem::transmute::<
                unsafe extern "C" fn(Object, c_int, Object),
                unsafe extern "C" fn(),
            >(record_reply),
            (&reply as *const AtomicI32).cast_mut().cast(),
            None,
            0,
        );
        assert_ne!(handler, 0);
        let username = *api._libraries[0]
            .get::<unsafe extern "C" fn(Object) -> *const c_char>(
                b"g_mount_operation_get_username\0",
            )
            .unwrap();
        let domain = *api._libraries[0]
            .get::<unsafe extern "C" fn(Object) -> *const c_char>(b"g_mount_operation_get_domain\0")
            .unwrap();
        let password = *api._libraries[0]
            .get::<unsafe extern "C" fn(Object) -> *const c_char>(
                b"g_mount_operation_get_password\0",
            )
            .unwrap();
        let save = *api._libraries[0]
            .get::<unsafe extern "C" fn(Object) -> c_int>(b"g_mount_operation_get_password_save\0")
            .unwrap();
        ask_password(&api, operation, 15);
        let field = |ptr: *const c_char| {
            assert!(!ptr.is_null());
            std::ffi::CStr::from_ptr(ptr).to_bytes()
        };
        assert_eq!(field(username(operation)), "uživatel".as_bytes());
        assert_eq!(field(domain(operation)), b"WORKGROUP");
        assert_eq!(field(password(operation)), b"fixture-password");
        assert_eq!(save(operation), 0);
        assert_eq!(reply.load(Ordering::Acquire), 0);
        assert!(mount.check().is_ok());

        // A second prompt means the same credentials were insufficient and
        // requires a new user submission.
        ask_password(&api, operation, 15);
        assert!(password(operation).is_null());
        assert_eq!(reply.load(Ordering::Acquire), 1);
        assert_eq!(
            mount.check().unwrap_err().code,
            ErrorCode::AuthenticationRequired
        );
    }
}
