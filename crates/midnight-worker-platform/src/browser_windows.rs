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

/// Bound for the remaining Chromium processes to exit after their job is terminated.
const TREE_EXIT: Duration = Duration::from_secs(2);

/// The broker and its whole process tree, which share one kill-on-close job.
pub struct Process {
    process: OwnedHandle,
    job: OwnedHandle,
}

impl Process {
    /// Terminate every process of the browser job. Termination completes asynchronously.
    pub fn kill(&mut self) -> io::Result<()> {
        // SAFETY: the owned job remains live; only the browser tree belongs to it.
        check(unsafe { TerminateJobObject(self.job.as_raw_handle(), 1) })
    }

    /// Wait for the broker, then end and await every remaining process of its job, so the
    /// session profile is no longer in use when this returns (as Linux awaits its cgroup).
    /// `Ok(false)` reports a nonzero broker exit status.
    pub fn wait(&mut self) -> io::Result<bool> {
        let code = wait_process(&self.process)?;
        // A broker may exit before its GPU, renderer and utility descendants.
        self.kill()?;
        wait_job_empty(&self.job, Instant::now() + TREE_EXIT)?;
        Ok(code == 0)
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        // A dropped supervisor cannot leave browser processes running in the background.
        let _ = self.kill();
        let _ = self.wait();
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
    host_shortcuts: bool,
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
    let mut environment = Vec::<u16>::new();
    for key in [
        "LOCALAPPDATA",
        super::HOST_SHORTCUTS_ENV,
        "MIDNIGHT_WEB_VIEWER_PROFILE",
        "SystemRoot",
        "TEMP",
        "TMP",
    ] {
        let value = match key {
            super::HOST_SHORTCUTS_ENV => {
                host_shortcuts.then(|| super::HOST_SHORTCUTS_PHYSICAL_V1.into())
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
    let process = Process { process, job };
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
