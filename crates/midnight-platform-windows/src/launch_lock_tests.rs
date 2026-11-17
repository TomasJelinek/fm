//! Editor and Custom Command program launches let the child inherit every inheritable handle,
//! so they must wait while a worker or browser creation holds the process-creation lock (its
//! child handles are inheritable meanwhile), and must release the lock before their child exits.
use super::*;
use midnight_core::{Cancellation, UserCommandService};
use std::{
    ffi::{OsStr, c_void},
    fs::File,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    path::PathBuf,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{NTSTATUS, STATUS_INFO_LENGTH_MISMATCH, WAIT_OBJECT_0},
    System::{
        Threading::{
            GetCurrentProcessId, GetCurrentThreadId, OpenProcess, PROCESS_SYNCHRONIZE,
            WaitForSingleObject,
        },
        WindowsProgramming::{SYSTEM_PROCESS_INFORMATION, SYSTEM_THREAD_INFORMATION},
    },
};

// `NtQuerySystemInformation` is in windows-sys `Wdk_System_SystemInformation`, which the crate
// does not enable for this one test call; its documented structures are already available.
#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtQuerySystemInformation(
        class: i32,
        information: *mut c_void,
        length: u32,
        returned: *mut u32,
    ) -> NTSTATUS;
}

const SYSTEM_PROCESS_INFORMATION_CLASS: i32 = 5;
/// `KTHREAD_STATE` Waiting with `KWAIT_REASON` WrAlertByThreadId: a thread blocked in
/// `WaitOnAddress`, where the futex-based `std::sync::Mutex` behind the lock waits.
const WAITING: u32 = 5;
const WR_ALERT_BY_THREAD_ID: u32 = 37;
const DEADLINE: Duration = Duration::from_secs(10);
const GATE: &str = "launch-lock-gate.txt";
const FIXTURE: &str = "editor::launch_lock_tests::gated_child_fixture";

/// Whether thread `id` of this process is blocked in `WaitOnAddress`, read from one
/// `SystemProcessInformation` snapshot (the structures documented in winternl.h).
fn waits_on_address(id: u32) -> bool {
    let mut buffer = vec![0u64; 1 << 17];
    loop {
        let mut returned = 0;
        let length = u32::try_from(buffer.len() * size_of::<u64>()).unwrap();
        // SAFETY: the buffer is writable for `length` bytes and 8-byte aligned, as the records
        // require; `returned` is a valid output.
        let status = unsafe {
            NtQuerySystemInformation(
                SYSTEM_PROCESS_INFORMATION_CLASS,
                buffer.as_mut_ptr().cast(),
                length,
                &mut returned,
            )
        };
        if status == STATUS_INFO_LENGTH_MISMATCH {
            // Processes may start before the next call; leave some room for them.
            buffer.resize(
                (returned as usize).div_ceil(size_of::<u64>()) + (1 << 14),
                0,
            );
            continue;
        }
        assert!(status >= 0, "NtQuerySystemInformation failed: {status:#x}");
        break;
    }
    // SAFETY: returns the calling process identifier.
    let process_id = unsafe { GetCurrentProcessId() } as usize;
    let base = buffer.as_ptr().cast::<u8>();
    let mut offset = 0;
    loop {
        // SAFETY: the kernel wrote a chain of 8-byte aligned process records inside the buffer,
        // linked by `NextEntryOffset`; each is followed by `NumberOfThreads` thread records.
        let (process, threads) = unsafe {
            let process = &*base.add(offset).cast::<SYSTEM_PROCESS_INFORMATION>();
            let threads = std::slice::from_raw_parts(
                base.add(offset + size_of::<SYSTEM_PROCESS_INFORMATION>())
                    .cast::<SYSTEM_THREAD_INFORMATION>(),
                process.NumberOfThreads as usize,
            );
            (process, threads)
        };
        if process.UniqueProcessId as usize == process_id {
            return threads.iter().any(|thread| {
                thread.ClientId.UniqueThread as usize == id as usize
                    && thread.ThreadState == WAITING
                    && thread.WaitReason == WR_ALERT_BY_THREAD_ID
            });
        }
        if process.NextEntryOffset == 0 {
            return false;
        }
        offset += process.NextEntryOffset as usize;
    }
}

/// Run `launch` on its own thread while this test holds the creation lock, as a worker or
/// browser creation does while its child handles are inheritable. The launch must block in
/// the lock instead of spawning; it spawns once the lock is released.
fn launch_during_a_locked_creation(
    launch: impl FnOnce() -> Result<Box<dyn TerminalProcess>, FsError> + Send + 'static,
) -> Box<dyn TerminalProcess> {
    let creation = midnight_worker_platform::lock_process_creation();
    let (identity, launcher_identity) = mpsc::channel();
    let launcher = thread::spawn(move || {
        // SAFETY: returns the calling thread identifier.
        identity.send(unsafe { GetCurrentThreadId() }).unwrap();
        launch()
    });
    let launcher_identity = launcher_identity.recv().unwrap();
    let deadline = Instant::now() + DEADLINE;
    while !launcher.is_finished() && !waits_on_address(launcher_identity) {
        assert!(
            Instant::now() < deadline,
            "the launch neither waited nor returned"
        );
        thread::yield_now();
    }
    assert!(
        !launcher.is_finished(),
        "the launch spawned while a creation held the lock"
    );
    drop(creation);
    launcher.join().unwrap().unwrap()
}

/// The lock is free again while `child` still runs, so worker creations are not held back
/// for the child's lifetime.
fn assert_lock_free_while_running(child: &mut dyn TerminalProcess) {
    let (acquired, lock) = mpsc::channel();
    thread::spawn(move || {
        drop(midnight_worker_platform::lock_process_creation());
        let _ = acquired.send(());
    });
    lock.recv_timeout(DEADLINE)
        .expect("the launch kept the creation lock after its spawn");
    assert!(
        child.try_wait().unwrap().is_none(),
        "the child exited before the gate was released"
    );
}

/// Wait for `child` to exit, bounded by a deadline that elapses only on failure.
fn exit_status(child: &mut dyn TerminalProcess) -> std::process::ExitStatus {
    // SAFETY: the boxed Child keeps its own process handle open, so the identifier cannot be
    // reused; ownership of the returned handle is taken only when it is valid.
    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, child.id()) };
    assert!(!process.is_null(), "{}", io::Error::last_os_error());
    // SAFETY: a valid process handle owned by nothing else.
    let process = unsafe { OwnedHandle::from_raw_handle(process) };
    let timeout = u32::try_from(DEADLINE.as_millis()).unwrap();
    // SAFETY: a live owned process handle.
    let waited = unsafe { WaitForSingleObject(process.as_raw_handle(), timeout) };
    assert_eq!(waited, WAIT_OBJECT_0, "the child did not exit");
    child
        .try_wait()
        .unwrap()
        .expect("an exited child has a status")
}

/// A file the child locks shared after it starts; it waits while the test holds it exclusively.
fn locked_gate(directory: &Path) -> (PathBuf, File) {
    let path = directory.join(GATE);
    let file = File::create(&path).unwrap();
    file.lock().unwrap();
    (path, file)
}

#[test]
fn editor_launch_waits_for_a_locked_creation_and_releases_the_lock_while_the_editor_runs() {
    let directory = crate::test_fixtures::build_temp("fixture-");
    let (file, gate) = locked_gate(directory.path());
    let preferences = EditorPreferences {
        rules: Vec::new(),
        executable: Some(std::env::current_exe().unwrap()),
        arguments: vec![
            "--exact".into(),
            FIXTURE.into(),
            "--ignored".into(),
            "{file}".into(),
        ],
    };
    let mut child = launch_during_a_locked_creation(move || {
        WindowsFileSystem::new(None).launch_editor(&file, &preferences)
    });
    assert_lock_free_while_running(child.as_mut());
    drop(gate);
    assert!(exit_status(child.as_mut()).success());
}

#[test]
fn program_launch_waits_for_a_locked_creation_and_releases_the_lock_while_the_program_runs() {
    let directory = crate::test_fixtures::build_temp("fixture-");
    let (file, gate) = locked_gate(directory.path());
    let cwd = directory.path().to_owned();
    let mut child = launch_during_a_locked_creation(move || {
        WindowsFileSystem::new(None).launch_user_program(
            &std::env::current_exe().unwrap(),
            &[
                "--exact".into(),
                FIXTURE.into(),
                "--ignored".into(),
                file.into_os_string(),
            ],
            &cwd,
            &Cancellation::default(),
        )
    });
    assert_lock_free_while_running(child.as_mut());
    drop(gate);
    assert!(exit_status(child.as_mut()).success());
}

#[test]
#[ignore = "Owned subprocess fixture; started only by the launch lock tests."]
fn gated_child_fixture() {
    // The tests above pass the gate as the last argument; any other run returns at once.
    let Some(gate) = std::env::args_os()
        .last()
        .map(PathBuf::from)
        .filter(|path| path.file_name() == Some(OsStr::new(GATE)))
    else {
        return;
    };
    File::open(gate).unwrap().lock_shared().unwrap();
}
