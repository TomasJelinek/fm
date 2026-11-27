//! Trusted CEF broker in a kill-on-close job; CEF owns its nested renderer sandbox jobs.
use super::Browser;
use crate::platform::{
    Attributes, check, create_with_inherited, pipe, suppress_error_dialogs, wait_process,
};
use std::{
    fs::OpenOptions,
    io,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::Path,
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    System::{JobObjects::*, Threading::*},
};

/// Bound for the whole Chromium tree, the broker included, to exit after the first
/// termination of its job. Termination is asynchronous: on a loaded machine
/// (`viewers_controls` beside three other acceptance scenarios) a terminated broker's
/// handle was signalled after up to 8 s, and once not within 34 s, while its job already
/// reported no active process.
const TREE_EXIT: Duration = Duration::from_secs(2);

/// The broker and its whole process tree, which share one kill-on-close job.
pub struct Process {
    process: OwnedHandle,
    job: OwnedHandle,
    /// When the job was first terminated: every later wait shares the one `TREE_EXIT`
    /// bound from then, so a wait after a kill, repeated by the drop, never waits longer.
    terminated: Option<Instant>,
}

impl Process {
    /// The broker's process ID, for diagnostics.
    pub fn id(&self) -> u32 {
        // SAFETY: the owned process handle stays live for this owner's lifetime.
        unsafe { GetProcessId(self.process.as_raw_handle()) }
    }

    /// Terminate every process of the browser job. Termination completes asynchronously.
    pub fn kill(&mut self) -> io::Result<()> {
        // Before the call: even a failed termination request bounds the waits after it.
        self.terminated.get_or_insert_with(Instant::now);
        // SAFETY: the owned job remains live; only the browser tree belongs to it.
        check(unsafe { TerminateJobObject(self.job.as_raw_handle(), 1) })
    }

    /// Wait for the broker, then end and await every remaining process of its job, so the
    /// session profile is no longer in use when this returns (as Linux awaits its cgroup).
    /// `Ok(false)` reports a nonzero broker exit status. After a [`kill`](Self::kill), the
    /// broker and the rest of its tree get `TREE_EXIT` from that first termination
    /// together: a tree still exiting then is reported as `TimedOut` instead of holding the
    /// viewer's worker. Its processes run no more code; the kill-on-close job is closed
    /// with this owner, and the profile they still hold is removed by a later session's
    /// sweep. A broker that was never killed is awaited without a bound.
    pub fn wait(&mut self) -> io::Result<bool> {
        let code = match self.terminated {
            Some(terminated) => wait_process_until(&self.process, terminated + TREE_EXIT)?,
            None => wait_process(&self.process)?,
        };
        // A broker may exit before its GPU, renderer and utility descendants.
        self.kill()?;
        let deadline = self.terminated.unwrap_or_else(Instant::now) + TREE_EXIT;
        wait_job_empty(&self.job, deadline)?;
        Ok(code == 0)
    }

    /// Why a browser that ended before its first message could not start. Windows names
    /// no cause here: a missing DLL ends the broker like any other startup failure.
    pub fn startup_failure(&mut self) -> Option<io::Error> {
        None
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        // A dropped supervisor cannot leave browser processes running in the background.
        let _ = self.kill();
        let _ = self.wait();
    }
}

/// Wait for a terminated broker to exit, until `deadline`, and return its exit code.
fn wait_process_until(process: &OwnedHandle, deadline: Instant) -> io::Result<u32> {
    let left = deadline.saturating_duration_since(Instant::now());
    // Whole milliseconds, rounded up, so a wait never ends before the deadline.
    let milliseconds = u32::try_from(left.as_micros().div_ceil(1000)).unwrap_or(INFINITE - 1);
    // SAFETY: an owned live process handle; the wait is bounded.
    match unsafe { WaitForSingleObject(process.as_raw_handle(), milliseconds) } {
        WAIT_OBJECT_0 => {
            let mut code = 1;
            // SAFETY: the same live handle; `code` is a valid output location.
            check(unsafe { GetExitCodeProcess(process.as_raw_handle(), &mut code) })?;
            Ok(code)
        }
        WAIT_TIMEOUT => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "Chromium broker still running after tree termination",
        )),
        _ => Err(io::Error::last_os_error()),
    }
}

/// Wait until the job reports no active process. Termination is asynchronous, so a broker
/// handle being signalled does not mean its descendants have released the profile files.
fn wait_job_empty(job: &OwnedHandle, deadline: Instant) -> io::Result<()> {
    loop {
        let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        // SAFETY: a live job handle and an exactly sized, writable information structure.
        check(unsafe {
            QueryInformationJobObject(
                job.as_raw_handle(),
                JobObjectBasicAccountingInformation,
                (&mut accounting as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                size_of_val(&accounting) as u32,
                null_mut(),
            )
        })?;
        if accounting.ActiveProcesses == 0 {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Chromium job still contains processes after tree termination",
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Whether a session's recording process may still run. Only a definitely missing process
/// counts as ended; denied access or any other failure keeps its session.
pub(super) fn process_alive(pid: u32) -> bool {
    // SAFETY: query/wait access only; ownership is taken only after success.
    let raw = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            pid,
        )
    };
    if raw.is_null() {
        return io::Error::last_os_error().raw_os_error() != Some(ERROR_INVALID_PARAMETER as i32);
    }
    // SAFETY: OpenProcess returned a new owned handle.
    let process = unsafe { OwnedHandle::from_raw_handle(raw) };
    // SAFETY: a live process handle; a zero timeout never blocks.
    unsafe { WaitForSingleObject(process.as_raw_handle(), 0) == WAIT_TIMEOUT }
}

pub(super) fn profile_parent() -> io::Result<std::path::PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")
        .ok_or_else(|| io::Error::other("LOCALAPPDATA unavailable"))?;
    let base = std::path::PathBuf::from(base);
    if !base.is_absolute() {
        return Err(io::Error::other("Invalid LOCALAPPDATA"));
    }
    Ok(base.join("MidnightExplorer").join("web-viewer-sessions"))
}

pub(super) fn spawn(
    executable: &Path,
    profile: super::profile::Profile,
    extensions: super::Extensions,
) -> io::Result<Browser> {
    let wide: Vec<u16> = executable.as_os_str().encode_wide().chain([0]).collect();
    if wide[..wide.len() - 1].contains(&0) {
        return Err(io::Error::other("NUL in browser path"));
    }
    // Every handle starts non-inheritable; only the creation call itself sees the child ends.
    let (child_input, input) = pipe()?;
    let (output, child_output) = pipe()?;
    let error = OpenOptions::new().write(true).open("NUL")?;
    let inherited = [
        child_input.as_raw_handle(),
        child_output.as_raw_handle(),
        error.as_raw_handle(),
    ];
    let mut attributes = Attributes::new()?;
    attributes.set(PROC_THREAD_ATTRIBUTE_HANDLE_LIST, &inherited)?;
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = inherited[0];
    startup.StartupInfo.hStdOutput = inherited[1];
    startup.StartupInfo.hStdError = inherited[2];
    startup.lpAttributeList = attributes.0.as_mut_ptr().cast();
    // SAFETY: a new unnamed job; ownership acquired only after success.
    let job = unsafe { CreateJobObjectW(null(), null()) };
    if job.is_null() {
        return Err(io::Error::last_os_error());
    }
    let job = unsafe { OwnedHandle::from_raw_handle(job) };
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        | JOB_OBJECT_LIMIT_ACTIVE_PROCESS
        | JOB_OBJECT_LIMIT_JOB_MEMORY;
    limits.BasicLimitInformation.ActiveProcessLimit = 32;
    limits.JobMemoryLimit = 2 * 1024 * 1024 * 1024;
    // SAFETY: exact-sized job data and owned handle. No breakaway permits orphaned children.
    check(unsafe {
        SetInformationJobObject(
            job.as_raw_handle(),
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of_val(&limits) as u32,
        )
    })?;
    // Explicit environment allowlist: no user tokens, proxy credentials, PATH or source names.
    // Names stay sorted case-insensitively, as Windows expects of an environment block.
    let mut environment = Vec::<u16>::new();
    for key in [
        "LOCALAPPDATA",
        super::CURSOR_ENV,
        super::HOST_SHORTCUTS_ENV,
        super::LINKS_ENV,
        "MIDNIGHT_WEB_VIEWER_PROFILE",
        "SystemRoot",
        "TEMP",
        "TMP",
    ] {
        let value = match key {
            super::CURSOR_ENV | super::HOST_SHORTCUTS_ENV | super::LINKS_ENV => {
                extensions.value(key).map(Into::into)
            }
            "MIDNIGHT_WEB_VIEWER_PROFILE" | "TEMP" | "TMP" => {
                Some(profile.path().as_os_str().to_owned())
            }
            _ => std::env::var_os(key),
        };
        if let Some(value) = value {
            if value.encode_wide().any(|unit| unit == 0) {
                return Err(io::Error::other("Invalid environment"));
            }
            environment.extend(key.encode_utf16());
            environment.push(b'=' as u16);
            environment.extend(value.encode_wide());
            environment.push(0);
        }
    }
    environment.push(0);
    if environment.len() == 1 {
        environment.push(0);
    }
    let directory: Vec<_> = executable
        .parent()
        .ok_or_else(|| io::Error::other("Missing parent"))?
        .as_os_str()
        .encode_wide()
        .chain([0])
        .collect();
    let mut child = PROCESS_INFORMATION::default();
    create_with_inherited([child_input, child_output, error], || {
        // SAFETY: explicit image, no shell/arguments, restricted handle list, suspended before
        // its job assignment.
        check(unsafe {
            CreateProcessW(
                wide.as_ptr(),
                null_mut(),
                null(),
                null(),
                1,
                EXTENDED_STARTUPINFO_PRESENT
                    | CREATE_UNICODE_ENVIRONMENT
                    | CREATE_SUSPENDED
                    | CREATE_NO_WINDOW,
                environment.as_ptr().cast(),
                directory.as_ptr(),
                &startup.StartupInfo,
                &mut child,
            )
        })
    })?;
    // SAFETY: CreateProcess succeeded and transferred both handles to this scope.
    let process = unsafe { OwnedHandle::from_raw_handle(child.hProcess) };
    let thread = unsafe { OwnedHandle::from_raw_handle(child.hThread) };
    // SAFETY: child is still suspended; terminate it if containment cannot be established.
    if let Err(error) =
        check(unsafe { AssignProcessToJobObject(job.as_raw_handle(), process.as_raw_handle()) })
    {
        unsafe {
            TerminateProcess(process.as_raw_handle(), 1);
            WaitForSingleObject(process.as_raw_handle(), 5000);
        }
        return Err(error);
    }
    let process = Process {
        process,
        job,
        terminated: None,
    };
    // Like the workers: a broker that cannot start ends with the loader's status, and CEF's
    // own processes inherit the broker's error mode. The guard kills the suspended broker.
    suppress_error_dialogs(&process.process)?;
    // SAFETY: owned primary thread of the suspended broker; the process guard kills on failure.
    if unsafe { ResumeThread(thread.as_raw_handle()) } == u32::MAX {
        return Err(io::Error::last_os_error());
    }
    Ok(Browser {
        input,
        output,
        process,
        _profile: profile,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    fn job() -> OwnedHandle {
        // SAFETY: a new unnamed job; ownership acquired only after success.
        let raw = unsafe { CreateJobObjectW(null(), null()) };
        assert!(!raw.is_null());
        // SAFETY: CreateJobObjectW returned a new owned handle.
        unsafe { OwnedHandle::from_raw_handle(raw) }
    }

    /// Kills and reaps a fixture process even when an assertion fails.
    struct Fixture(std::process::Child);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn job_wait_returns_only_after_every_tree_process_has_exited() {
        use std::os::windows::process::CommandExt;
        let job = job();
        // An empty job is immediately empty.
        wait_job_empty(&job, Instant::now()).unwrap();
        // A suspended member never exits on its own, so the job stays populated.
        let child = Fixture(
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "no-such-test"])
                .creation_flags(CREATE_SUSPENDED)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        // SAFETY: the live owned job and child process handles.
        check(unsafe { AssignProcessToJobObject(job.as_raw_handle(), child.0.as_raw_handle()) })
            .unwrap();
        assert_eq!(
            wait_job_empty(&job, Instant::now()).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        // SAFETY: terminate only this test's job; then the bounded wait observes it empty.
        check(unsafe { TerminateJobObject(job.as_raw_handle(), 1) }).unwrap();
        wait_job_empty(&job, Instant::now() + Duration::from_secs(10)).unwrap();
    }

    /// A broker that has not exited `TREE_EXIT` after its tree was terminated no longer
    /// holds the caller: the wait reports `TimedOut`, and the drop's repeated kill and wait
    /// share that first deadline instead of waiting again. The broker here sits outside the
    /// job, so terminating the job leaves it running, as termination that does not complete
    /// (a loaded machine kept a real broker alive for more than 34 s) leaves the waiter.
    #[test]
    fn waits_after_a_termination_share_one_bound() {
        use std::os::windows::{io::AsHandle, process::CommandExt};
        let broker = Fixture(
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "no-such-test"])
                .creation_flags(CREATE_SUSPENDED)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let mut process = Process {
            process: broker.0.as_handle().try_clone_to_owned().unwrap(),
            job: job(),
            terminated: None,
        };
        let killed = Instant::now();
        process.kill().unwrap();
        let error = process.wait().unwrap_err();
        let waited = killed.elapsed();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut, "{error}");
        // Windows may end a timed wait up to a timer tick early.
        assert!(
            waited >= TREE_EXIT - Duration::from_millis(50),
            "{waited:?}"
        );
        assert!(waited < TREE_EXIT + Duration::from_secs(3), "{waited:?}");
        // The drop's kill and wait: the deadline has passed, so it returns at once.
        let dropped = Instant::now();
        drop(process);
        assert!(
            dropped.elapsed() < Duration::from_millis(500),
            "{:?}",
            dropped.elapsed()
        );
        // Still running: only the waits ended. The fixture ends it.
        assert!(process_alive(broker.0.id()));
    }

    /// A broker in its job exits on termination, and the wait confirms the whole tree.
    #[test]
    fn a_terminated_broker_in_its_job_is_confirmed_well_within_the_bound() {
        use std::os::windows::{io::AsHandle, process::CommandExt};
        let broker = Fixture(
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "no-such-test"])
                .creation_flags(CREATE_SUSPENDED)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let job = job();
        // SAFETY: the live owned job and child process handles.
        check(unsafe { AssignProcessToJobObject(job.as_raw_handle(), broker.0.as_raw_handle()) })
            .unwrap();
        let mut process = Process {
            process: broker.0.as_handle().try_clone_to_owned().unwrap(),
            job,
            terminated: None,
        };
        process.kill().unwrap();
        // Terminated with exit code 1, so `Ok(false)`.
        assert!(!process.wait().unwrap());
        assert!(!process_alive(broker.0.id()));
    }

    #[test]
    fn only_definitely_missing_processes_are_reported_as_ended() {
        assert!(process_alive(std::process::id()));
        let mut child = Fixture(
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "no-such-test"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let pid = child.0.id();
        child.0.wait().unwrap();
        // The retained process handle reserves the ID, so no new process can reuse it: the
        // exited process still opens and its zero-timeout wait reports that it has ended.
        assert!(!process_alive(pid));
        drop(child);
        // An ID beyond any the system assigns is definitely missing (ERROR_INVALID_PARAMETER).
        assert!(!process_alive(0xffff_fffc));
    }
}
