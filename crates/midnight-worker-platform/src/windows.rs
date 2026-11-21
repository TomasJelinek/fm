//! Capability-free AppContainer; only explicitly listed pipe handles are inherited.
//!
//! Every handle is created non-inheritable. A child's three standard handles become
//! inheritable only for its `CreateProcessW` call, under the process-creation lock, and are
//! closed before the lock is released, so launches that inherit all inheritable handles
//! (`std::process::Command`) cannot capture them unless they overlap that single call.
use crate::{CpuBudget, ExitStatus, ResourcePolicy, Worker};
use std::{
    ffi::c_void,
    fs::{File, OpenOptions},
    io,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::Path,
    ptr::{null, null_mut},
    sync::{Mutex, MutexGuard},
};
use windows_sys::Win32::{
    Foundation::*,
    Security::{FreeSid, Isolation::*, SECURITY_ATTRIBUTES, SECURITY_CAPABILITIES},
    System::{
        Diagnostics::Debug::{
            SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX, SEM_NOOPENFILEERRORBOX,
        },
        JobObjects::*,
        Pipes::CreatePipe,
        SystemInformation::GetSystemWindowsDirectoryW,
        Threading::*,
    },
};

/// The error mode every worker and browser process starts with, as `SetErrorMode` would set
/// it: no "System Error" dialog when the loader cannot find or load a library (the process
/// ends with that status instead), no Windows Error Reporting dialog after a crash and no
/// dialog for a file that cannot be opened.
const CHILD_ERROR_MODE: u32 =
    SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX | SEM_NOOPENFILEERRORBOX;
/// `ProcessDefaultHardErrorMode` of the native process information classes.
const PROCESS_DEFAULT_HARD_ERROR_MODE: u32 = 12;

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtSetInformationProcess(
        process: HANDLE,
        class: u32,
        information: *const c_void,
        length: u32,
    ) -> NTSTATUS;
}

/// Give a suspended child [`CHILD_ERROR_MODE`] before any of its code runs, its loader
/// included. A child inherits only its parent's process error mode (a `SetThreadErrorMode`
/// around `CreateProcessW` is not inherited), and the application keeps the default, so a
/// worker whose loader could not open a library otherwise waited on a modal "System Error"
/// dialog. The native value stores `SEM_FAILCRITICALERRORS` inverted, as `SetErrorMode`
/// does: there, bit 0 set means "show critical-error dialogs".
pub(super) fn suppress_error_dialogs(process: &OwnedHandle) -> io::Result<()> {
    let native = CHILD_ERROR_MODE ^ SEM_FAILCRITICALERRORS;
    // SAFETY: a live process handle that CreateProcessW returned with full access; the input
    // is a valid u32 for the call's duration.
    let status = unsafe {
        NtSetInformationProcess(
            process.as_raw_handle(),
            PROCESS_DEFAULT_HARD_ERROR_MODE,
            (&native as *const u32).cast(),
            size_of::<u32>() as u32,
        )
    };
    if status < 0 {
        return Err(io::Error::other(format!(
            "Cannot turn off system error dialogs for a worker process (NTSTATUS 0x{:08X})",
            status.cast_unsigned()
        )));
    }
    Ok(())
}

/// The loader's status for a library it could not find, which it also reports when the
/// worker was denied access to the library it found.
pub(super) fn library_not_loaded(status: ExitStatus) -> bool {
    status == ExitStatus::Code(0xC000_0135)
}

pub(super) fn library_name(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("dll"))
}

/// Whether the workers' AppContainer may open `path` as a worker's loader opens a library
/// (read data and execute). Authz evaluates the file's security descriptor for this user's
/// token restricted to the workers' AppContainer SID, as Windows checks both for the worker.
pub(super) fn worker_may_load(path: &Path) -> io::Result<bool> {
    use windows_sys::Win32::{
        Security::{
            Authorization::*, DACL_SECURITY_INFORMATION, GROUP_SECURITY_INFORMATION,
            OWNER_SECURITY_INFORMATION, TOKEN_QUERY,
        },
        Storage::FileSystem::{FILE_EXECUTE, FILE_READ_DATA, SYNCHRONIZE},
    };
    struct Owned<T: Copy>(T, unsafe fn(T));
    impl<T: Copy> Drop for Owned<T> {
        fn drop(&mut self) {
            // SAFETY: each guard holds a handle that its successful native call returned.
            unsafe { (self.1)(self.0) };
        }
    }
    unsafe fn free_local(memory: *mut c_void) {
        // SAFETY: memory that Windows allocated for this caller with LocalAlloc.
        unsafe { LocalFree(memory) };
    }
    unsafe fn free_context(context: AUTHZ_CLIENT_CONTEXT_HANDLE) {
        // SAFETY: a context initialized below.
        unsafe { AuthzFreeContext(context) };
    }
    unsafe fn free_manager(manager: AUTHZ_RESOURCE_MANAGER_HANDLE) {
        // SAFETY: a resource manager initialized below.
        unsafe { AuthzFreeResourceManager(manager) };
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    if wide[..wide.len() - 1].contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "NUL in library path",
        ));
    }
    let sid = worker_profile_sid()?;
    let mut descriptor = null_mut();
    // SAFETY: a terminated path; Windows allocates the descriptor, freed by its guard.
    let status = unsafe {
        GetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let descriptor = Owned(descriptor, free_local);
    let mut manager = null_mut();
    // SAFETY: no callbacks or name; the output is writable.
    check(unsafe {
        AuthzInitializeResourceManager(
            AUTHZ_RM_FLAG_NO_AUDIT,
            None,
            None,
            None,
            null(),
            &mut manager,
        )
    })?;
    let manager = Owned(manager, free_manager);
    let mut token = null_mut();
    // SAFETY: this process's pseudo handle; ownership is acquired only on success.
    check(unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) })?;
    // SAFETY: a token handle that the call above returned.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut context = null_mut();
    // SAFETY: a live token and resource manager; a zero identifier and no dynamic groups.
    check(unsafe {
        AuthzInitializeContextFromToken(
            0,
            token.as_raw_handle(),
            manager.0,
            null(),
            LUID::default(),
            null(),
            &mut context,
        )
    })?;
    let context = Owned(context, free_context);
    // SAFETY: a live context and the workers' AppContainer SID; no capabilities.
    check(unsafe { AuthzSetAppContainerInformation(context.0, sid.0, 0, null()) })?;
    let wanted = FILE_READ_DATA | FILE_EXECUTE | SYNCHRONIZE;
    let request = AUTHZ_ACCESS_REQUEST {
        DesiredAccess: wanted,
        ..Default::default()
    };
    let (mut granted, mut error) = (0u32, 0u32);
    let mut reply = AUTHZ_ACCESS_REPLY {
        ResultListLength: 1,
        GrantedAccessMask: &mut granted,
        SaclEvaluationResults: null_mut(),
        Error: &mut error,
    };
    // SAFETY: a live context, request and descriptor; the reply points to two live u32s.
    check(unsafe {
        AuthzAccessCheck(
            0,
            context.0,
            &request,
            null_mut(),
            descriptor.0,
            null(),
            0,
            &mut reply,
            null_mut(),
        )
    })?;
    Ok(error == 0 && granted & wanted == wanted)
}

/// [`ExitStatus::reason`] on Windows: the statuses a worker ends with when its loader cannot
/// start it, when it crashes or when it runs out of a resource, in words with the status's
/// name and code; any other exit code as hexadecimal.
pub(super) fn exit_reason(status: ExitStatus) -> String {
    let code = match status {
        ExitStatus::Code(code) => code,
        ExitStatus::Signal(signal) => return format!("it was terminated by signal {signal}"),
    };
    let known = match code {
        0xC000_0135 => (
            "a required library is missing or cannot be read",
            "STATUS_DLL_NOT_FOUND",
        ),
        0xC000_0139 => (
            "a required library does not provide a function it needs",
            "STATUS_ENTRYPOINT_NOT_FOUND",
        ),
        0xC000_0142 => (
            "a required library could not start",
            "STATUS_DLL_INIT_FAILED",
        ),
        0xC000_007B => (
            "one of its program files is damaged or for another system",
            "STATUS_INVALID_IMAGE_FORMAT",
        ),
        0xC000_0022 => (
            "it was denied access to a file it needs",
            "STATUS_ACCESS_DENIED",
        ),
        0xC000_0005 => ("it crashed", "STATUS_ACCESS_VIOLATION"),
        0xC000_001D => (
            "it crashed on an unsupported processor instruction",
            "STATUS_ILLEGAL_INSTRUCTION",
        ),
        0xC000_00FD => ("it ran out of stack space", "STATUS_STACK_OVERFLOW"),
        0xC000_0409 => (
            "it stopped on an internal error",
            "STATUS_STACK_BUFFER_OVERRUN",
        ),
        0xC000_0017 => ("it ran out of memory", "STATUS_NO_MEMORY"),
        0xC000_012D => ("it ran out of memory", "STATUS_COMMITMENT_LIMIT"),
        _ => return format!("it exited with code 0x{code:08X}"),
    };
    format!("{} ({}, 0x{code:08X})", known.0, known.1)
}

pub struct Process {
    pub(super) process: OwnedHandle,
    pub(super) job: OwnedHandle,
}

impl Process {
    /// Positively wait for termination regardless of the expected forced-shutdown exit
    /// status. Unlike wait(), a nonzero exit is not an unconfirmed native wait.
    pub fn wait_terminated(&mut self) -> io::Result<()> {
        // SAFETY: this owner retains the live process handle throughout the blocking wait.
        if unsafe { WaitForSingleObject(self.process.as_raw_handle(), INFINITE) } != WAIT_OBJECT_0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    pub fn kill(&mut self) -> io::Result<()> {
        // SAFETY: the owned job remains live and contains only this worker.
        check(unsafe { TerminateJobObject(self.job.as_raw_handle(), 1) })
    }

    /// Wait for the worker to exit; `Ok(false)` reports a nonzero exit status, as on Linux.
    pub fn wait(&mut self) -> io::Result<bool> {
        self.wait_status().map(ExitStatus::success)
    }

    /// Wait for the worker to exit and return its full 32-bit exit code.
    pub fn wait_status(&mut self) -> io::Result<ExitStatus> {
        wait_process(&self.process).map(ExitStatus::Code)
    }
}

/// Wait for a process to exit and return its exit code.
pub(super) fn wait_process(process: &OwnedHandle) -> io::Result<u32> {
    let mut code = 1;
    // SAFETY: process is an owned live process handle; output points to a valid u32.
    unsafe {
        if WaitForSingleObject(process.as_raw_handle(), INFINITE) != WAIT_OBJECT_0 {
            return Err(io::Error::last_os_error());
        }
        check(GetExitCodeProcess(process.as_raw_handle(), &mut code))?;
    }
    Ok(code)
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

/// Both ends start non-inheritable; see [`create_with_inherited`].
pub(super) fn pipe() -> io::Result<(File, File)> {
    let mut read = null_mut();
    let mut write = null_mut();
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: null_mut(),
        bInheritHandle: 0,
    };
    // SAFETY: output handles and attributes are valid; ownership is acquired only on success.
    unsafe {
        check(CreatePipe(&mut read, &mut write, &attributes, 64 * 1024))?;
        Ok((File::from_raw_handle(read), File::from_raw_handle(write)))
    }
}

static PROCESS_CREATION: Mutex<()> = Mutex::new(());

/// Serializes acquiring the per-user AppContainer profile: concurrent first creation of the
/// same profile can fail with E_UNEXPECTED (seen on a fresh CI user while workers started in
/// parallel).
static WORKER_PROFILE: Mutex<()> = Mutex::new(());

/// The SID of the restricted workers' AppContainer, creating its per-user profile when absent.
/// A concurrent creator in another process is given a few short retries.
fn worker_profile_sid() -> io::Result<Sid> {
    const ALREADY_EXISTS: i32 = 0x800700b7u32 as i32;
    const UNEXPECTED: i32 = 0x8000ffffu32 as i32;
    let name: Vec<u16> = "MidnightExplorer.RestrictedWorker.v1"
        .encode_utf16()
        .chain([0])
        .collect();
    let _profile = WORKER_PROFILE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut result = UNEXPECTED;
    for attempt in 0..5 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(10 << attempt));
        }
        let mut sid = null_mut();
        // SAFETY: fixed terminated name, no granted capabilities, and writable SID output.
        // Windows requires the per-user profile to exist; merely deriving its SID makes
        // CreateProcess fail.
        result = unsafe {
            CreateAppContainerProfile(
                name.as_ptr(),
                name.as_ptr(),
                name.as_ptr(),
                null(),
                0,
                &mut sid,
            )
        };
        if result == ALREADY_EXISTS {
            // SAFETY: the same terminated name and a writable SID output.
            result = unsafe { DeriveAppContainerSidFromAppContainerName(name.as_ptr(), &mut sid) };
        }
        if result >= 0 {
            return Ok(Sid(sid));
        }
        if result != UNEXPECTED {
            break;
        }
    }
    Err(io::Error::other(format!(
        "AppContainer SID unavailable: {result:#x}"
    )))
}

/// Serializes the short intervals in which this process holds handles marked inheritable for
/// one specific child. In-process launchers that let `CreateProcessW` inherit every inheritable
/// handle, including `std::process::Command::spawn`, should hold this guard while spawning so
/// that they cannot capture a worker's pipe ends.
pub fn lock_process_creation() -> MutexGuard<'static, ()> {
    PROCESS_CREATION
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Run `create` (one `CreateProcessW` restricted by `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`) while
/// exactly `handles` are inheritable. The handles are closed before the creation lock is
/// released on every exit path: success, a failed `SetHandleInformation`, a failed `create`
/// and unwinding out of it. The child holds its own duplicates; no inheritable copy remains in
/// this process afterwards.
pub(super) fn create_with_inherited<H: AsRawHandle, const N: usize, R>(
    handles: [H; N],
    create: impl FnOnce() -> io::Result<R>,
) -> io::Result<R> {
    let _creation = lock_process_creation();
    // Locals drop in reverse declaration order and before parameters, so this rebinding closes
    // the handles while `_creation` still holds the lock, whichever way the function exits.
    let handles = handles;
    for handle in &handles {
        // SAFETY: an owned live handle; only its inheritance flag changes. An early return
        // closes every handle under the creation lock, so none stays inheritable outside it.
        check(unsafe {
            SetHandleInformation(
                handle.as_raw_handle(),
                HANDLE_FLAG_INHERIT,
                HANDLE_FLAG_INHERIT,
            )
        })?;
    }
    create()
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
    // Non-inheritable like the pipes until the creation call itself.
    let error = OpenOptions::new().write(true).open("NUL")?;
    #[cfg(feature = "startup-diagnostics")]
    timing.mark("pipes");
    let sid = worker_profile_sid()?;
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
    create_with_inherited([child_input, child_output, error], || {
        // SAFETY: explicit executable avoids command parsing/search; no arguments or secret
        // environment. The attribute list assigns the resource-limited job before successful
        // creation; suspended start prevents execution until the process guard is installed.
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
    // A worker that cannot start (a library it needs is missing) ends with the loader's
    // status, which the host reports; it never waits on a system dialog.
    suppress_error_dialogs(&process.process)?;
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
#[path = "windows_startup_tests.rs"]
mod startup_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        os::windows::io::RawHandle,
        panic::AssertUnwindSafe,
        process::{Child, ChildStderr, Command, Stdio},
        sync::{
            Arc, TryLockError,
            atomic::{AtomicBool, Ordering},
        },
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{
        Security::{GetTokenInformation, TOKEN_QUERY, TokenIsAppContainer},
        System::Pipes::PeekNamedPipe,
    };

    fn inheritable(handle: HANDLE) -> bool {
        let mut flags = 0;
        // SAFETY: a live handle owned by the calling test; the output is a valid u32.
        check(unsafe { GetHandleInformation(handle, &mut flags) }).unwrap();
        flags & HANDLE_FLAG_INHERIT != 0
    }

    #[test]
    fn pipe_ends_start_non_inheritable() {
        // Concurrent std::process::Command launches inherit every inheritable handle.
        let (read, write) = pipe().unwrap();
        assert!(!inheritable(read.as_raw_handle()));
        assert!(!inheritable(write.as_raw_handle()));
    }

    #[test]
    fn child_handles_are_inheritable_only_inside_the_locked_creation_call() {
        let (read, write) = pipe().unwrap();
        let (kept, other) = pipe().unwrap();
        let raw = [read.as_raw_handle(), write.as_raw_handle()];
        create_with_inherited([read, write], || {
            assert!(
                PROCESS_CREATION.try_lock().is_err(),
                "other launchers must wait"
            );
            assert!(raw.iter().all(|handle| inheritable(*handle)));
            // Handles that are not passed in are never made inheritable.
            assert!(!inheritable(kept.as_raw_handle()));
            assert!(!inheritable(other.as_raw_handle()));
            Ok(())
        })
        .unwrap();
        // The creation lock is free again; the passed handles were closed while holding it.
        drop(lock_process_creation());
        // A failed creation also releases the lock and leaves nothing inheritable behind.
        let (read, write) = pipe().unwrap();
        assert!(
            create_with_inherited([read, write], || Err::<(), _>(io::Error::other("fixture")))
                .is_err()
        );
        drop(lock_process_creation());
    }

    #[test]
    fn unrelated_spawns_hold_the_creation_lock_only_while_spawning() {
        use std::os::windows::process::CommandExt;
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "no-such-test"])
            .creation_flags(CREATE_SUSPENDED)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = crate::spawn_command(&mut command).unwrap();
        // The suspended child keeps running, yet the lock is free again. Other tests may hold
        // it for a moment, so retry until a deadline that elapses only on failure.
        let deadline = Instant::now() + Duration::from_secs(10);
        let acquired = loop {
            match PROCESS_CREATION.try_lock() {
                Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::yield_now()
                }
                other => break !matches!(other, Err(TryLockError::WouldBlock)),
            }
        };
        let _ = child.kill();
        let _ = child.wait();
        assert!(acquired, "the creation lock outlived the spawn call");
    }

    /// A child handle that records whether the creation lock was still held when it closed.
    struct Probe {
        file: Option<File>,
        closed_under_lock: Arc<AtomicBool>,
    }

    impl Probe {
        fn new(file: Option<File>) -> (Self, Arc<AtomicBool>) {
            let closed_under_lock = Arc::new(AtomicBool::new(false));
            let probe = Self {
                file,
                closed_under_lock: closed_under_lock.clone(),
            };
            (probe, closed_under_lock)
        }
    }

    impl AsRawHandle for Probe {
        fn as_raw_handle(&self) -> RawHandle {
            // Without a file, the null handle makes SetHandleInformation fail.
            self.file
                .as_ref()
                .map_or(null_mut(), AsRawHandle::as_raw_handle)
        }
    }

    impl Drop for Probe {
        fn drop(&mut self) {
            // WouldBlock while any thread, including this one, holds the lock. A free (even
            // poisoned) lock is acquired and released again at once.
            let held = matches!(PROCESS_CREATION.try_lock(), Err(TryLockError::WouldBlock));
            self.closed_under_lock.store(held, Ordering::SeqCst);
            drop(self.file.take());
        }
    }

    #[test]
    fn child_handles_are_closed_before_the_creation_lock_on_every_exit_path() {
        let (read, write) = pipe().unwrap();
        // Success and a failed creation.
        for result in [Ok(()), Err(io::Error::other("fixture"))] {
            let (probe, closed) = Probe::new(Some(read.try_clone().unwrap()));
            let created = create_with_inherited([probe], || result);
            assert!(closed.load(Ordering::SeqCst));
            drop(created);
        }
        // SetHandleInformation fails on the second handle after marking the first one.
        let (marked, marked_closed) = Probe::new(Some(read.try_clone().unwrap()));
        let (invalid, invalid_closed) = Probe::new(None);
        let refused = create_with_inherited([marked, invalid], || -> io::Result<()> {
            unreachable!("creation must not run after a failed inheritance change")
        });
        assert!(refused.is_err());
        assert!(marked_closed.load(Ordering::SeqCst));
        assert!(invalid_closed.load(Ordering::SeqCst));
        // Unwinding out of the creation call.
        let (probe, closed) = Probe::new(Some(write.try_clone().unwrap()));
        let unwound = std::panic::catch_unwind(AssertUnwindSafe(|| {
            create_with_inherited([probe], || -> io::Result<()> { panic!("fixture") })
        }));
        assert!(unwound.is_err());
        assert!(closed.load(Ordering::SeqCst));
        // The panic poisoned the lock; production callers recover, other tests start clean.
        PROCESS_CREATION.clear_poison();
        drop(lock_process_creation());
    }

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
