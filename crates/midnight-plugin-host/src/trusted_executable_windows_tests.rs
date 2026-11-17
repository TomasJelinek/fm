use super::{ExecutableLease, test_support};
use crate::{CapturedFile, TrustedExecutablePin};
use std::{
    cell::Cell,
    fs::{self, File, OpenOptions},
    io,
    os::windows::{
        fs::OpenOptionsExt,
        io::{AsRawHandle, RawHandle},
    },
    path::{Path, PathBuf},
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION, GetHandleInformation,
        HANDLE_FLAG_INHERIT,
    },
    Storage::FileSystem::{
        DELETE, FILE_BASIC_INFO, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_WRITE_ATTRIBUTES,
        FILE_WRITE_DATA, FileBasicInfo, GetFileInformationByHandleEx, SetFileInformationByHandle,
    },
};

const ORIGINAL: &[u8] = b"worker original";
const REPLACEMENT: &[u8] = b"worker replaced";
const SHARE_ALL: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;

struct Fixture {
    _root: tempfile::TempDir,
    package: PathBuf,
    executable: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let base = std::env::var_os("HOST_TEST_INSTALL_ROOT")
            .expect("protected normal-profile HOST_TEST_INSTALL_ROOT");
        let root = tempfile::tempdir_in(base).unwrap();
        crate::trust::protect_fixture(root.path()).unwrap();
        let package = root.path().join("worker-package");
        fs::create_dir(&package).unwrap();
        let executable = package.join("worker.exe");
        fs::write(&executable, ORIGINAL).unwrap();
        Self {
            package: package.canonicalize().unwrap(),
            executable: executable.canonicalize().unwrap(),
            _root: root,
        }
    }

    fn capture(&self) -> TrustedExecutablePin {
        TrustedExecutablePin::capture(&self.package, &self.executable).unwrap()
    }
}

fn open_access(path: &Path, access: u32) -> io::Result<File> {
    OpenOptions::new()
        .access_mode(access)
        .share_mode(SHARE_ALL)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

fn assert_sharing_violation<T>(operation: &str, result: io::Result<T>) {
    let error = match result {
        Ok(_) => panic!("{operation}: the retained executable lease allowed a conflicting open"),
        Err(error) => error,
    };
    assert_eq!(
        error.raw_os_error(),
        Some(ERROR_SHARING_VIOLATION as i32),
        "{operation}: {error}"
    );
}

fn assert_mutation_denied(operation: &str, result: io::Result<()>) {
    let error = result.expect_err(operation);
    assert!(
        matches!(error.raw_os_error(), Some(code) if code == ERROR_ACCESS_DENIED as i32 || code == ERROR_SHARING_VIOLATION as i32),
        "{operation}: {error}",
    );
}

fn basic_info(file: &File) -> FILE_BASIC_INFO {
    let mut info = FILE_BASIC_INFO::default();
    // SAFETY: the File owns this live handle and info is writable for its exact size.
    assert_ne!(
        unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileBasicInfo,
                (&mut info as *mut FILE_BASIC_INFO).cast(),
                size_of::<FILE_BASIC_INFO>() as u32,
            )
        },
        0,
        "{}",
        io::Error::last_os_error()
    );
    info
}

fn set_basic_info(file: &File, update: &FILE_BASIC_INFO) {
    // SAFETY: the File owns this live handle and update is initialized and readable
    // for exactly FILE_BASIC_INFO's size. The call does not retain the pointer.
    assert_ne!(
        unsafe {
            SetFileInformationByHandle(
                file.as_raw_handle(),
                FileBasicInfo,
                (update as *const FILE_BASIC_INFO).cast(),
                size_of::<FILE_BASIC_INFO>() as u32,
            )
        },
        0,
        "{}",
        io::Error::last_os_error()
    );
}

#[test]
fn metadata_only_change_time_is_accepted_only_by_the_retained_executable_pin() {
    let fixture = Fixture::new();
    let pin = fixture.capture();
    let generic = CapturedFile::capture(&fixture.executable).unwrap();
    generic.revalidate().unwrap();
    let metadata = open_access(
        &fixture.executable,
        FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES,
    )
    .unwrap();
    let before_basic = basic_info(&metadata);
    let before = crate::native::snapshot(&metadata).unwrap();
    let change_time = before_basic.ChangeTime.checked_add(10_000_000).unwrap();
    let update = FILE_BASIC_INFO {
        ChangeTime: change_time,
        ..Default::default()
    };
    // Zero leaves every other field unchanged under FileBasicInfo semantics.
    set_basic_info(&metadata, &update);
    let after_basic = basic_info(&metadata);
    assert_eq!(after_basic.ChangeTime, change_time);
    let after = crate::native::snapshot(&metadata).unwrap();
    assert_eq!(
        before.changed_fields(&after),
        format!(
            "ChangeTime(before={}, after={change_time})",
            before_basic.ChangeTime
        ),
        "this fixture must change only ChangeTime",
    );
    pin.revalidate().unwrap();
    let error = generic.revalidate().unwrap_err();
    assert!(
        error
            .0
            .starts_with("Source changed since capture (LiveValidated revision)"),
        "{error}",
    );
    assert!(error.0.contains("stage=open"), "{error}");
    assert_eq!(fs::read(&fixture.executable).unwrap(), ORIGINAL);
    drop(metadata);
    pin.release_on_worker().unwrap();
}

#[test]
fn metadata_only_last_write_time_change_is_rejected() {
    let fixture = Fixture::new();
    let pin = fixture.capture();
    let metadata = open_access(
        &fixture.executable,
        FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES,
    )
    .unwrap();
    let write_time = basic_info(&metadata)
        .LastWriteTime
        .checked_add(20_000_000)
        .unwrap();
    set_basic_info(
        &metadata,
        &FILE_BASIC_INFO {
            LastWriteTime: write_time,
            ..Default::default()
        },
    );
    assert_eq!(basic_info(&metadata).LastWriteTime, write_time);
    let error = pin.revalidate().unwrap_err();
    assert!(error.0.contains("LastWriteTime("), "{error}");
    assert_eq!(fs::read(&fixture.executable).unwrap(), ORIGINAL);
    drop(metadata);
    pin.release_on_worker().unwrap();
}

#[test]
fn alternate_data_stream_cannot_be_captured_as_the_executable() {
    let fixture = Fixture::new();
    let alternate = fixture.package.join("worker.exe:other");
    fs::write(&alternate, ORIGINAL).unwrap();
    assert!(TrustedExecutablePin::capture(&fixture.package, &alternate).is_err());
    assert!(ExecutableLease::capture(&alternate).is_err());
    fixture.capture().release_on_worker().unwrap();
}

#[test]
fn retained_pin_denies_data_write_delete_rename_and_replacement_until_release() {
    let fixture = Fixture::new();
    let replacement = fixture.package.join("replacement.exe");
    let moved = fixture.package.join("moved.exe");
    fs::write(&replacement, REPLACEMENT).unwrap();
    let pin = fixture.capture();
    assert_sharing_violation(
        "open WriteData while pinned",
        open_access(&fixture.executable, FILE_WRITE_DATA),
    );
    assert_sharing_violation(
        "open DELETE while pinned",
        open_access(&fixture.executable, DELETE),
    );
    assert_mutation_denied(
        "remove pinned executable",
        fs::remove_file(&fixture.executable),
    );
    assert_mutation_denied(
        "rename pinned executable",
        fs::rename(&fixture.executable, &moved),
    );
    assert_mutation_denied(
        "replace pinned executable",
        fs::rename(&replacement, &fixture.executable),
    );
    assert_eq!(fs::read(&fixture.executable).unwrap(), ORIGINAL);
    assert_eq!(fs::read(&replacement).unwrap(), REPLACEMENT);
    assert!(
        !moved.try_exists().unwrap(),
        "denied rename must not create its destination"
    );
    pin.revalidate().unwrap();
    pin.release_on_worker().unwrap();
    drop(open_access(&fixture.executable, FILE_WRITE_DATA).unwrap());
    drop(open_access(&fixture.executable, DELETE).unwrap());
    fs::rename(&fixture.executable, &moved).unwrap();
    assert!(!fixture.executable.try_exists().unwrap());
    assert_eq!(fs::read(&moved).unwrap(), ORIGINAL);
    fs::rename(&moved, &fixture.executable).unwrap();
    assert!(!moved.try_exists().unwrap());
    assert_eq!(fs::read(&fixture.executable).unwrap(), ORIGINAL);
    fs::rename(&replacement, &fixture.executable).unwrap();
    assert!(!replacement.try_exists().unwrap());
    assert_eq!(fs::read(&fixture.executable).unwrap(), REPLACEMENT);
    fs::remove_file(&fixture.executable).unwrap();
    assert!(!fixture.executable.try_exists().unwrap());
}

#[test]
fn preexisting_data_writer_prevents_capture_even_when_it_shares_all_access() {
    let fixture = Fixture::new();
    let writer = open_access(&fixture.executable, FILE_WRITE_DATA).unwrap();
    assert!(TrustedExecutablePin::capture(&fixture.package, &fixture.executable).is_err());
    drop(writer);
    fixture.capture().release_on_worker().unwrap();
}

#[test]
fn retained_handle_is_not_inheritable() {
    let fixture = Fixture::new();
    let pin = fixture.capture();
    let handle = test_support::raw_handle(&pin.pin);
    let mut flags = 0;
    // SAFETY: pin retains the handle throughout this read-only flag query.
    assert_ne!(unsafe { GetHandleInformation(handle, &mut flags) }, 0);
    assert_eq!(flags & HANDLE_FLAG_INHERIT, 0);
    pin.release_on_worker().unwrap();
}

#[test]
fn held_identity_cannot_validate_a_fresh_file_with_matching_length_and_write_time() {
    let fixture = Fixture::new();
    let replacement = fixture.package.join("fresh.exe");
    fs::write(&replacement, ORIGINAL).unwrap();
    let modified = fs::metadata(&fixture.executable)
        .unwrap()
        .modified()
        .unwrap();
    File::options()
        .write(true)
        .open(&replacement)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    assert_eq!(
        fs::metadata(&replacement).unwrap().len(),
        ORIGINAL.len() as u64
    );
    assert_eq!(
        fs::metadata(&replacement).unwrap().modified().unwrap(),
        modified
    );
    let lease = ExecutableLease::capture(&fixture.executable).unwrap();
    lease.revalidate(&fixture.executable).unwrap();
    assert!(lease.revalidate(&replacement).is_err());
    lease.release_on_worker().unwrap();
    fs::rename(&replacement, &fixture.executable).unwrap();
    fixture.capture().release_on_worker().unwrap();
}

thread_local! {
    static CLOSE_CALLS: Cell<usize> = const { Cell::new(0) };
}

fn failed_close(_: RawHandle) -> io::Result<()> {
    CLOSE_CALLS.with(|calls| calls.set(calls.get() + 1));
    Err(io::Error::other("injected unconfirmed native close"))
}

fn panicking_close(_: RawHandle) -> io::Result<()> {
    CLOSE_CALLS.with(|calls| calls.set(calls.get() + 1));
    panic!("injected native close unwind")
}

fn assert_unconfirmed_close_retains_handle(drop_fallback: bool, unwind: bool) {
    let fixture = Fixture::new();
    let lease = ExecutableLease::capture(&fixture.executable).unwrap();
    let handle = test_support::raw_handle(&lease);
    CLOSE_CALLS.with(|calls| calls.set(0));
    let close = if unwind {
        panicking_close
    } else {
        failed_close
    };
    let result = std::panic::catch_unwind(move || {
        if drop_fallback {
            test_support::drop_with(lease, close);
            None
        } else {
            Some(test_support::close_with(lease, close))
        }
    });
    if unwind {
        assert!(result.is_err());
    } else if !drop_fallback {
        assert!(result.unwrap().unwrap().is_err());
    } else {
        assert!(result.unwrap().is_none());
    }
    assert_eq!(
        CLOSE_CALLS.with(Cell::get),
        1,
        "Drop must not retry an uncertain close"
    );
    assert_sharing_violation(
        "open WriteData after unconfirmed held close",
        open_access(&fixture.executable, FILE_WRITE_DATA),
    );
    let renamed = fixture.package.join("renamed.exe");
    assert_mutation_denied(
        "rename after unconfirmed held close",
        fs::rename(&fixture.executable, &renamed),
    );
    assert_eq!(fs::read(&fixture.executable).unwrap(), ORIGINAL);
    assert!(!renamed.try_exists().unwrap());
    // SAFETY: the injected hook never closed this uniquely retained native handle.
    // This is test-only cleanup after proving production deliberately retained it.
    assert_ne!(unsafe { CloseHandle(handle) }, 0);
    drop(open_access(&fixture.executable, FILE_WRITE_DATA).unwrap());
    fs::rename(&fixture.executable, &renamed).unwrap();
    assert_eq!(fs::read(&renamed).unwrap(), ORIGINAL);
    fs::remove_file(renamed).unwrap();
}

#[test]
fn explicit_close_error_retains_unknown_handle_without_drop_retry() {
    assert_unconfirmed_close_retains_handle(false, false);
}

#[test]
fn explicit_close_unwind_retains_unknown_handle_without_drop_retry() {
    assert_unconfirmed_close_retains_handle(false, true);
}

#[test]
fn fallback_drop_close_error_retains_unknown_handle_without_retry() {
    assert_unconfirmed_close_retains_handle(true, false);
}

#[test]
fn fallback_drop_close_unwind_retains_unknown_handle_without_retry() {
    assert_unconfirmed_close_retains_handle(true, true);
}

thread_local! {
    static UNKNOWN_PROBE: Cell<RawHandle> = const { Cell::new(std::ptr::null_mut()) };
    static PROBE_CLOSE_CALLS: Cell<usize> = const { Cell::new(0) };
    static HELD_CLOSE_CALLS: Cell<usize> = const { Cell::new(0) };
    static HELD_CLOSE_CONFIRMED: Cell<bool> = const { Cell::new(false) };
}

fn native_close_for_test(handle: RawHandle) -> io::Result<()> {
    // SAFETY: the test hook receives the single pending owner's live handle.
    // The pending owner disables File::drop before invoking this close callback.
    if unsafe { CloseHandle(handle) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn fail_first_probe_close(handle: RawHandle) -> io::Result<()> {
    let calls = PROBE_CLOSE_CALLS.with(|calls| {
        let next = calls.get() + 1;
        calls.set(next);
        next
    });
    if calls == 1 {
        UNKNOWN_PROBE.with(|probe| probe.set(handle));
        Err(io::Error::other("injected unknown named-probe close"))
    } else {
        native_close_for_test(handle)
    }
}

fn unwind_first_probe_close(handle: RawHandle) -> io::Result<()> {
    let calls = PROBE_CLOSE_CALLS.with(|calls| {
        let next = calls.get() + 1;
        calls.set(next);
        next
    });
    if calls == 1 {
        UNKNOWN_PROBE.with(|probe| probe.set(handle));
        panic!("injected unknown named-probe close unwind");
    }
    native_close_for_test(handle)
}

fn confirm_held_close(handle: RawHandle) -> io::Result<()> {
    HELD_CLOSE_CALLS.with(|calls| calls.set(calls.get() + 1));
    native_close_for_test(handle)?;
    HELD_CLOSE_CONFIRMED.with(|confirmed| confirmed.set(true));
    Ok(())
}

fn assert_unknown_probe_prevents_later_validation_and_confirmed_retirement(unwind: bool) {
    let fixture = Fixture::new();
    let mut pin = fixture.capture();
    let held = test_support::raw_handle(&pin.pin);
    let probes_before = test_support::probe_opens(&pin.pin);
    UNKNOWN_PROBE.with(|probe| probe.set(std::ptr::null_mut()));
    PROBE_CLOSE_CALLS.with(|calls| calls.set(0));
    HELD_CLOSE_CALLS.with(|calls| calls.set(0));
    HELD_CLOSE_CONFIRMED.with(|confirmed| confirmed.set(false));
    test_support::set_probe_close(
        &mut pin.pin,
        if unwind {
            unwind_first_probe_close
        } else {
            fail_first_probe_close
        },
    );
    test_support::set_held_close(&mut pin.pin, confirm_held_close);

    let first = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| pin.revalidate()));
    if unwind {
        assert!(first.is_err());
    } else {
        assert!(first.unwrap().is_err());
    }
    let unknown = UNKNOWN_PROBE.with(Cell::get);
    assert!(
        !unknown.is_null(),
        "the injected hook must observe a real probe handle"
    );
    assert_ne!(
        unknown, held,
        "the named probe must be a separate native owner"
    );
    assert_eq!(test_support::probe_opens(&pin.pin), probes_before + 1);
    for _ in 0..2 {
        assert!(
            pin.revalidate().is_err(),
            "an unknown probe close must remain latched"
        );
        assert_eq!(
            test_support::probe_opens(&pin.pin),
            probes_before + 1,
            "a latched pin must reject revalidation before another probe open",
        );
        assert_eq!(PROBE_CLOSE_CALLS.with(Cell::get), 1);
    }

    assert_eq!(HELD_CLOSE_CALLS.with(Cell::get), 0);
    assert!(!HELD_CLOSE_CONFIRMED.with(Cell::get));
    let error = pin.release_on_worker().unwrap_err();
    assert!(error.0.contains("unconfirmed"), "{error}");
    assert_eq!(HELD_CLOSE_CALLS.with(Cell::get), 1);
    assert!(
        HELD_CLOSE_CONFIRMED.with(Cell::get),
        "the held owner must actually close"
    );
    assert_eq!(
        PROBE_CLOSE_CALLS.with(Cell::get),
        1,
        "the unknown close must not be retried"
    );
    assert_sharing_violation(
        "open WriteData after unconfirmed probe close",
        open_access(&fixture.executable, FILE_WRITE_DATA),
    );
    let renamed = fixture.package.join("renamed-after-unknown-probe.exe");
    assert_mutation_denied(
        "rename after unconfirmed probe close",
        fs::rename(&fixture.executable, &renamed),
    );
    assert_eq!(fs::read(&fixture.executable).unwrap(), ORIGINAL);
    assert!(!renamed.try_exists().unwrap());

    // SAFETY: the injected first-probe hook deliberately never called native close.
    // The held owner has closed; only this known-open test probe is being cleaned up.
    assert_ne!(unsafe { CloseHandle(unknown) }, 0);
    UNKNOWN_PROBE.with(|probe| probe.set(std::ptr::null_mut()));
    drop(open_access(&fixture.executable, FILE_WRITE_DATA).unwrap());
    fs::rename(&fixture.executable, &renamed).unwrap();
    assert_eq!(fs::read(&renamed).unwrap(), ORIGINAL);
    fs::remove_file(renamed).unwrap();
}

#[test]
fn named_probe_close_error_sticks_before_reopen_and_prevents_confirmed_retirement() {
    assert_unknown_probe_prevents_later_validation_and_confirmed_retirement(false);
}

#[test]
fn named_probe_close_unwind_sticks_before_reopen_and_prevents_confirmed_retirement() {
    assert_unknown_probe_prevents_later_validation_and_confirmed_retirement(true);
}
