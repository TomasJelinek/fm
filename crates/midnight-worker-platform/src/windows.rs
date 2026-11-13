//! Capability-free AppContainer; only explicitly listed pipe handles are inherited.
use crate::{CpuBudget, ResourcePolicy, Worker};
use std::{
    fs::{File, OpenOptions},
    io,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::Path,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::*,
    Security::{FreeSid, Isolation::*, SECURITY_ATTRIBUTES, SECURITY_CAPABILITIES},
    System::{
        JobObjects::*, Pipes::CreatePipe, SystemInformation::GetSystemWindowsDirectoryW,
        Threading::*,
    },
};

pub struct Process {
    pub(super) process: OwnedHandle,
    pub(super) job: OwnedHandle,
}

impl Process {
    pub fn kill(&mut self) -> io::Result<()> {
        // SAFETY: the owned job remains live and contains only this worker.
        check(unsafe { TerminateJobObject(self.job.as_raw_handle(), 1) })
    }

    pub fn wait(&mut self) -> io::Result<bool> {
        let mut code = 1;
        // SAFETY: process is an owned live process handle; output points to a valid u32.
        unsafe {
            if WaitForSingleObject(self.process.as_raw_handle(), INFINITE) != WAIT_OBJECT_0 {
                return Err(io::Error::last_os_error());
            }
            check(GetExitCodeProcess(self.process.as_raw_handle(), &mut code))?;
        }
        if code != 0 {
            return Err(io::Error::other(format!(
                "Restricted worker exited with code {code:#x}"
            )));
        }
        Ok(true)
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        // A dropped supervisor cannot leave a decoder running in the background.
        let _ = self.kill();
        let _ = self.wait();
    }
}

pub(super) fn check(value: i32) -> io::Result<()> {
    if value == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub(super) fn pipe() -> io::Result<(File, File)> {
    let mut read = null_mut();
    let mut write = null_mut();
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: null_mut(),
        bInheritHandle: 1,
    };
    // SAFETY: output handles and attributes are valid; ownership is acquired only on success.
    unsafe {
        check(CreatePipe(&mut read, &mut write, &attributes, 64 * 1024))?;
        Ok((File::from_raw_handle(read), File::from_raw_handle(write)))
    }
}

struct Sid(*mut std::ffi::c_void);
impl Drop for Sid {
    fn drop(&mut self) {
        // SAFETY: DeriveAppContainerSidFromAppContainerName allocated this SID with FreeSid contract.
        unsafe {
            FreeSid(self.0);
        }
    }
}

pub(super) struct Attributes(pub(super) Vec<usize>);
impl Attributes {
    pub(super) fn new() -> io::Result<Self> {
        let mut bytes = 0;
        // SAFETY: first call only obtains size; second has sufficiently aligned writable storage.
        unsafe {
            InitializeProcThreadAttributeList(null_mut(), 4, 0, &mut bytes);
            let mut storage = vec![0usize; bytes.div_ceil(size_of::<usize>())];
            check(InitializeProcThreadAttributeList(
                storage.as_mut_ptr().cast(),
                4,
                0,
                &mut bytes,
            ))?;
            Ok(Self(storage))
        }
    }
    pub(super) fn set<T>(&mut self, key: u32, value: &T) -> io::Result<()> {
        // SAFETY: backing storage is initialized, value is valid and retained through CreateProcess.
        check(unsafe {
            UpdateProcThreadAttribute(
                self.0.as_mut_ptr().cast(),
                0,
                key as usize,
                (value as *const T).cast(),
                size_of::<T>(),
                null_mut(),
                null(),
            )
        })
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        // SAFETY: attributes owns the initialized list and all child startup calls have finished.
        unsafe {
            DeleteProcThreadAttributeList(self.0.as_mut_ptr().cast());
        }
    }
}

pub(super) fn spawn(executable: &Path, limits: ResourcePolicy) -> io::Result<Worker> {
    spawn_impl(
        executable,
        limits,
        #[cfg(test)]
        None,
    )
}

#[cfg(test)]
type BeforeResume<'a> = dyn FnMut(&Process) -> io::Result<()> + 'a;

fn spawn_impl(
    executable: &Path,
    limits: ResourcePolicy,
    #[cfg(test)] before_resume: Option<&mut BeforeResume<'_>>,
) -> io::Result<Worker> {
    #[cfg(feature = "startup-diagnostics")]
    let mut timing = crate::startup_diagnostics::Trace::new("restricted_windows");
    let wide: Vec<_> = executable.as_os_str().encode_wide().chain([0]).collect();
    if wide[..wide.len() - 1].contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "NUL in worker path",
        ));
    }
    let (child_input, input) = pipe()?;
    let (output, child_output) = pipe()?;
    let error = OpenOptions::new().write(true).open("NUL")?;
    // SAFETY: files own live pipe/null handles; only the intended child ends remain inheritable.
    unsafe {
        check(SetHandleInformation(
            input.as_raw_handle(),
            HANDLE_FLAG_INHERIT,
            0,
        ))?;
        check(SetHandleInformation(
            output.as_raw_handle(),
            HANDLE_FLAG_INHERIT,
            0,
        ))?;
        check(SetHandleInformation(
            error.as_raw_handle(),
            HANDLE_FLAG_INHERIT,
            HANDLE_FLAG_INHERIT,
        ))?;
    }
    #[cfg(feature = "startup-diagnostics")]
    timing.mark("pipes");
    let name: Vec<u16> = "MidnightExplorer.RestrictedWorker.v1"
        .encode_utf16()
        .chain([0])
        .collect();
    let mut sid = null_mut();
    // SAFETY: fixed terminated name, no granted capabilities, and writable SID output. Windows
    // requires the per-user profile to exist; merely deriving its SID makes CreateProcess fail.
    let mut result = unsafe {
        CreateAppContainerProfile(
            name.as_ptr(),
            name.as_ptr(),
            name.as_ptr(),
            null(),
            0,
            &mut sid,
        )
    };
    if result == 0x800700b7u32 as i32 {
        result = unsafe { DeriveAppContainerSidFromAppContainerName(name.as_ptr(), &mut sid) };
    }
    if result < 0 {
        return Err(io::Error::other(format!(
            "AppContainer SID unavailable: {result:#x}"
        )));
    }
    let sid = Sid(sid);
    #[cfg(feature = "startup-diagnostics")]
    timing.mark("profile");
    // SAFETY: unnamed non-inheritable job; ownership begins only after a nonnull handle.
    let job = unsafe { CreateJobObjectW(null(), null()) };
    if job.is_null() {
        return Err(io::Error::last_os_error());
    }
    let job = unsafe { OwnedHandle::from_raw_handle(job) };
    let mut budget = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    budget.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        | JOB_OBJECT_LIMIT_ACTIVE_PROCESS
        | JOB_OBJECT_LIMIT_PROCESS_MEMORY;
    budget.BasicLimitInformation.ActiveProcessLimit = 1;
    if let CpuBudget::Seconds(seconds) = limits.cpu {
        budget.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_PROCESS_TIME;
        budget.BasicLimitInformation.PerProcessUserTimeLimit = i64::from(seconds) * 10_000_000;
    }
    budget.ProcessMemoryLimit = usize::try_from(limits.memory_bytes).map_err(io::Error::other)?;
    // SAFETY: exact sized job information structure on a live owned job.
    check(unsafe {
        SetInformationJobObject(
            job.as_raw_handle(),
            JobObjectExtendedLimitInformation,
            (&budget as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of_val(&budget) as u32,
        )
    })?;
    let capabilities = SECURITY_CAPABILITIES {
        AppContainerSid: sid.0,
        ..Default::default()
    };
    let inherited = [
        child_input.as_raw_handle(),
        child_output.as_raw_handle(),
        error.as_raw_handle(),
    ];
    // Keep the job-handle array alive until the attribute list is destroyed. This assigns the
    // configured inner job during creation without inheriting a job handle into the worker.
    let jobs = [job.as_raw_handle()];
    let child_policy: u32 = 1; // PROCESS_CREATION_CHILD_PROCESS_RESTRICTED
    let mut attributes = Attributes::new()?;
    attributes.set(PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, &capabilities)?;
    attributes.set(PROC_THREAD_ATTRIBUTE_HANDLE_LIST, &inherited)?;
    attributes.set(PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY, &child_policy)?;
    attributes.set(PROC_THREAD_ATTRIBUTE_JOB_LIST, &jobs)?;
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = child_input.as_raw_handle();
    startup.StartupInfo.hStdOutput = child_output.as_raw_handle();
    startup.StartupInfo.hStdError = error.as_raw_handle();
    startup.lpAttributeList = attributes.0.as_mut_ptr().cast();
    #[cfg(feature = "startup-diagnostics")]
    timing.mark("job");
    let mut process = PROCESS_INFORMATION::default();
    // AppContainer startup needs SystemRoot. Query the OS instead of inheriting user secrets/PATH.
    let mut system_root = vec![0u16; 32768];
    let length =
        unsafe { GetSystemWindowsDirectoryW(system_root.as_mut_ptr(), system_root.len() as u32) }
            as usize;
    if length == 0 || length >= system_root.len() {
        return Err(io::Error::last_os_error());
    }
    let local = std::env::var_os("LOCALAPPDATA").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "LOCALAPPDATA is unavailable for AppContainer startup",
        )
    })?;
    if !Path::new(&local).is_absolute() || local.encode_wide().any(|unit| unit == 0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid LOCALAPPDATA",
        ));
    }
    let mut environment: Vec<u16> = "LOCALAPPDATA=".encode_utf16().collect();
    environment.extend(local.encode_wide());
    environment.push(0);
    environment.extend("SystemRoot=".encode_utf16());
    environment.extend_from_slice(&system_root[..length]);
    environment.extend_from_slice(&[0, 0]);
    #[cfg(feature = "startup-diagnostics")]
    timing.mark("environment");
    // SAFETY: explicit executable avoids command parsing/search; no arguments or secret environment.
    // The attribute list assigns the resource-limited job before successful creation; suspended
    // start also prevents worker execution until the owned process guard is installed.
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
                | DETACHED_PROCESS,
            environment.as_ptr().cast(),
            null(),
            &startup.StartupInfo,
            &mut process,
        )
    })
    .map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("CreateProcessW for restricted worker: {error}"),
        )
    })?;
    #[cfg(feature = "startup-diagnostics")]
    timing.mark("process_created");
    // SAFETY: CreateProcess succeeded and transferred both handles to this scope.
    let process_handle = unsafe { OwnedHandle::from_raw_handle(process.hProcess) };
    let thread = unsafe { OwnedHandle::from_raw_handle(process.hThread) };
    let process = Process {
        process: process_handle,
        job,
    };
    #[cfg(test)]
    if let Some(observe) = before_resume {
        observe(&process)?;
    }
    // SAFETY: owned primary thread of the suspended worker; process guard kills on failure.
    if unsafe { ResumeThread(thread.as_raw_handle()) } == u32::MAX {
        return Err(io::Error::last_os_error());
    }
    #[cfg(feature = "startup-diagnostics")]
    timing.mark("contained_resumed");
    Ok(Worker {
        input,
        output,
        process,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        process::{Child, ChildStderr, Command, Stdio},
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{
        Security::{GetTokenInformation, TOKEN_QUERY, TokenIsAppContainer},
        System::Pipes::PeekNamedPipe,
    };

    const TEST_NAME: &str =
        "platform::tests::native_worker_job_is_assigned_before_resume_and_parent_exit_kills_child";
    const PARENT_ENV: &str = "MIDNIGHT_SUSPENDED_WORKER_PARENT_TEST";
    const REPORT_PREFIX: &[u8; 8] = b"MIDJOB01";
    const REPORT_BYTES: usize = REPORT_PREFIX.len() + 8 * 8;
    const MEMORY_BYTES: u64 = 512 * 1024 * 1024;

    // The existing self-executable subprocess pattern is also used by worker_policy_windows.
    // This observes inner membership before resume and kill-on-close after parent termination.
    // Creation-time assignment is established by JOB_LIST and the absence of later assignment
    // in the production diff; the observer also accepts a post-create assignment placed before it.
    // This covers an assigned suspended child after CreateProcess returns, before ResumeThread;
    // it does not cover interruption inside CreateProcess, Ready workers or running IPC sessions.
    #[test]
    #[ignore = "native AppContainer subprocess test; Build owns its protected runtime and lease"]
    fn native_worker_job_is_assigned_before_resume_and_parent_exit_kills_child() -> io::Result<()> {
        if let Ok(outer_pid) = std::env::var(PARENT_ENV) {
            return suspended_parent(outer_pid.parse().map_err(io::Error::other)?);
        }
        // SAFETY: a fresh unnamed cleanup job, owned only here, with no inherited handle.
        let cleanup_job = unsafe { CreateJobObjectW(null(), null()) };
        if cleanup_job.is_null() {
            return Err(io::Error::last_os_error());
        }
        let cleanup_job = unsafe { OwnedHandle::from_raw_handle(cleanup_job) };
        let mut cleanup_limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        cleanup_limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: the test owns the job and supplies its exact information structure.
        check(unsafe {
            SetInformationJobObject(
                cleanup_job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&cleanup_limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of_val(&cleanup_limits) as u32,
            )
        })?;
        let mut parent = ParentFixture(
            Command::new(std::env::current_exe()?)
                .args([
                    "--exact",
                    TEST_NAME,
                    "--ignored",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(PARENT_ENV, std::process::id().to_string())
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()?,
        );
        // SAFETY: this owned parent waits on its stdin gate before creating the worker.
        check(unsafe {
            AssignProcessToJobObject(cleanup_job.as_raw_handle(), parent.0.as_raw_handle())
        })?;
        parent.0.stdin.as_mut().unwrap().write_all(b"g")?;
        let mut report = parent.0.stderr.take().unwrap();
        let (worker, observed) = read_suspended_report(&parent.0, &mut report)?;
        assert_eq!(
            observed[0], 1,
            "child must already belong to its INNER worker job"
        );
        assert_eq!(observed[1] & u64::from(HANDLE_FLAG_INHERIT), 0);
        assert_eq!(
            observed[2],
            u64::from(
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
                    | JOB_OBJECT_LIMIT_ACTIVE_PROCESS
                    | JOB_OBJECT_LIMIT_PROCESS_MEMORY
                    | JOB_OBJECT_LIMIT_PROCESS_TIME
            )
        );
        assert_eq!(observed[3], 1);
        assert_eq!(observed[4], MEMORY_BYTES);
        assert_eq!(observed[5], 5 * 10_000_000);
        assert_eq!(
            observed[6], 1,
            "the suspended child must remain an AppContainer"
        );
        // SAFETY: both handles are owned and live; zero-time waits cannot execute the worker.
        assert_eq!(
            unsafe { WaitForSingleObject(worker.as_raw_handle(), 0) },
            WAIT_TIMEOUT
        );
        assert_eq!(
            unsafe { WaitForSingleObject(parent.0.as_raw_handle(), 0) },
            WAIT_TIMEOUT
        );
        // Only the owned parent PROCESS is terminated. The outer cleanup job stays open,
        // and no worker job handle was duplicated out of the parent to delay last-handle close.
        check(unsafe { TerminateProcess(parent.0.as_raw_handle(), 97) })?;
        assert_eq!(
            unsafe { WaitForSingleObject(parent.0.as_raw_handle(), 5_000) },
            WAIT_OBJECT_0,
            "owned parent did not exit"
        );
        let _ = parent.0.wait()?;
        let worker_exit = unsafe { WaitForSingleObject(worker.as_raw_handle(), 5_000) };
        assert_eq!(
            worker_exit, WAIT_OBJECT_0,
            "inner job must kill the suspended child while outer cleanup job is still open"
        );
        drop(cleanup_job);
        Ok(())
    }

    struct ParentFixture(Child);

    impl Drop for ParentFixture {
        fn drop(&mut self) {
            // Child::kill targets only this owned process. The separately owned outer job
            // closes afterward on unwind, so a failed assertion cannot leave its fixture alive.
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn suspended_parent(outer_pid: u32) -> io::Result<()> {
        std::io::stdin().read_exact(&mut [0])?;
        // This exact PID was supplied by the test process that owns this parent. It is used
        // only to duplicate a query/wait handle back to that owner, never for termination.
        let outer = unsafe { OpenProcess(PROCESS_DUP_HANDLE, 0, outer_pid) };
        if outer.is_null() {
            return Err(io::Error::last_os_error());
        }
        let outer = unsafe { OwnedHandle::from_raw_handle(outer) };
        let mut before_resume = |process: &Process| {
            let mut in_inner_job = 0;
            let mut job_handle_flags = 0;
            let mut budget = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            // SAFETY: query the concrete configured INNER job and the live suspended process.
            unsafe {
                check(IsProcessInJob(
                    process.process.as_raw_handle(),
                    process.job.as_raw_handle(),
                    &mut in_inner_job,
                ))?;
                check(GetHandleInformation(
                    process.job.as_raw_handle(),
                    &mut job_handle_flags,
                ))?;
                check(QueryInformationJobObject(
                    process.job.as_raw_handle(),
                    JobObjectExtendedLimitInformation,
                    (&mut budget as *mut JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    size_of_val(&budget) as u32,
                    null_mut(),
                ))?;
            }
            let mut token = null_mut();
            // SAFETY: acquire ownership only after a successful token-open call.
            check(unsafe {
                OpenProcessToken(process.process.as_raw_handle(), TOKEN_QUERY, &mut token)
            })?;
            let token = unsafe { OwnedHandle::from_raw_handle(token) };
            let mut app_container = 0u32;
            let mut returned = 0;
            // SAFETY: TokenIsAppContainer returns a DWORD into this correctly sized buffer.
            check(unsafe {
                GetTokenInformation(
                    token.as_raw_handle(),
                    TokenIsAppContainer,
                    (&mut app_container as *mut u32).cast(),
                    size_of_val(&app_container) as u32,
                    &mut returned,
                )
            })?;
            let mut remote_worker = null_mut();
            // SAFETY: source and destination processes are live. Only a non-inheritable
            // process query/wait handle is duplicated; the worker job remains parent-owned.
            check(unsafe {
                DuplicateHandle(
                    GetCurrentProcess(),
                    process.process.as_raw_handle(),
                    outer.as_raw_handle(),
                    &mut remote_worker,
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                    0,
                    0,
                )
            })?;
            let values = [
                remote_worker as usize as u64,
                in_inner_job as u64,
                u64::from(job_handle_flags),
                u64::from(budget.BasicLimitInformation.LimitFlags),
                u64::from(budget.BasicLimitInformation.ActiveProcessLimit),
                budget.ProcessMemoryLimit as u64,
                budget.BasicLimitInformation.PerProcessUserTimeLimit as u64,
                u64::from(app_container),
            ];
            let mut frame = Vec::from(*REPORT_PREFIX);
            for value in values {
                frame.extend_from_slice(&value.to_le_bytes());
            }
            let mut report = std::io::stderr().lock();
            report.write_all(&frame)?;
            report.flush()?;
            // The worker's primary thread remains suspended throughout this bounded fixture.
            // It executes no IPC, EOF handling or independent exit timer. The outer test has
            // 10 seconds to receive the report and 5 seconds to observe each process exit.
            std::thread::sleep(Duration::from_secs(60));
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "owned parent was not terminated",
            ))
        };
        let _ = spawn_impl(
            &std::env::current_exe()?,
            ResourcePolicy {
                memory_bytes: MEMORY_BYTES,
                cpu: CpuBudget::Seconds(5),
            },
            Some(&mut before_resume),
        )?;
        Err(io::Error::other("suspended fixture unexpectedly resumed"))
    }

    fn read_suspended_report(
        parent: &Child,
        report: &mut ChildStderr,
    ) -> io::Result<(OwnedHandle, [u64; 7])> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let mut available = 0;
            // SAFETY: this is the owned parent's pipe; only its byte count is queried.
            check(unsafe {
                PeekNamedPipe(
                    report.as_raw_handle(),
                    null_mut(),
                    0,
                    null_mut(),
                    &mut available,
                    null_mut(),
                )
            })?;
            if available as usize >= REPORT_BYTES {
                break;
            }
            // SAFETY: the owned parent handle remains live; the wait is nonblocking.
            if unsafe { WaitForSingleObject(parent.as_raw_handle(), 0) } != WAIT_TIMEOUT {
                return Err(io::Error::other(
                    "owned parent exited before the suspended report",
                ));
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "suspended worker report",
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut frame = [0u8; REPORT_BYTES];
        report.read_exact(&mut frame)?;
        if &frame[..REPORT_PREFIX.len()] != REPORT_PREFIX {
            return Err(io::Error::other("invalid suspended worker report"));
        }
        let mut values = [0u64; 8];
        for (value, bytes) in values
            .iter_mut()
            .zip(frame[REPORT_PREFIX.len()..].as_chunks::<8>().0)
        {
            *value = u64::from_le_bytes(*bytes);
        }
        let handle = usize::try_from(values[0]).map_err(io::Error::other)? as HANDLE;
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::other("invalid duplicated worker handle"));
        }
        // SAFETY: the fixed report transfers the sole remote duplicate created for this test.
        let worker = unsafe { OwnedHandle::from_raw_handle(handle) };
        Ok((worker, values[1..].try_into().unwrap()))
    }
}
