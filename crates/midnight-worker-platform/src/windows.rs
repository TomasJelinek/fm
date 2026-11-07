//! Capability-free AppContainer; only explicitly listed pipe handles are inherited.
use crate::{Limits, Worker};
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
    process: OwnedHandle,
    job: OwnedHandle,
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

fn check(value: i32) -> io::Result<()> {
    if value == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn pipe() -> io::Result<(File, File)> {
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

struct Attributes(Vec<usize>);
impl Attributes {
    fn new() -> io::Result<Self> {
        let mut bytes = 0;
        // SAFETY: first call only obtains size; second has sufficiently aligned writable storage.
        unsafe {
            InitializeProcThreadAttributeList(null_mut(), 3, 0, &mut bytes);
            let mut storage = vec![0usize; bytes.div_ceil(size_of::<usize>())];
            check(InitializeProcThreadAttributeList(
                storage.as_mut_ptr().cast(),
                3,
                0,
                &mut bytes,
            ))?;
            Ok(Self(storage))
        }
    }
    fn set<T>(&mut self, key: u32, value: &T) -> io::Result<()> {
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

pub(super) fn spawn(executable: &Path, limits: Limits) -> io::Result<Worker> {
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
    let capabilities = SECURITY_CAPABILITIES {
        AppContainerSid: sid.0,
        ..Default::default()
    };
    let inherited = [
        child_input.as_raw_handle(),
        child_output.as_raw_handle(),
        error.as_raw_handle(),
    ];
    let child_policy: u32 = 1; // PROCESS_CREATION_CHILD_PROCESS_RESTRICTED
    let mut attributes = Attributes::new()?;
    attributes.set(PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, &capabilities)?;
    attributes.set(PROC_THREAD_ATTRIBUTE_HANDLE_LIST, &inherited)?;
    attributes.set(PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY, &child_policy)?;
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = child_input.as_raw_handle();
    startup.StartupInfo.hStdOutput = child_output.as_raw_handle();
    startup.StartupInfo.hStdError = error.as_raw_handle();
    startup.lpAttributeList = attributes.0.as_mut_ptr().cast();
    // SAFETY: unnamed new job; ownership begins only after a nonnull successful handle.
    let job = unsafe { CreateJobObjectW(null(), null()) };
    if job.is_null() {
        return Err(io::Error::last_os_error());
    }
    let job = unsafe { OwnedHandle::from_raw_handle(job) };
    let mut budget = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    budget.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        | JOB_OBJECT_LIMIT_ACTIVE_PROCESS
        | JOB_OBJECT_LIMIT_PROCESS_MEMORY
        | JOB_OBJECT_LIMIT_PROCESS_TIME;
    budget.BasicLimitInformation.ActiveProcessLimit = 1;
    budget.BasicLimitInformation.PerProcessUserTimeLimit =
        i64::from(limits.cpu_seconds) * 10_000_000;
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
    // SAFETY: explicit executable avoids command parsing/search; no arguments or secret environment.
    // Suspended start prevents untrusted parsing before assignment to the resource-limited job.
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
    // SAFETY: CreateProcess succeeded and transferred both handles to this scope.
    let process_handle = unsafe { OwnedHandle::from_raw_handle(process.hProcess) };
    let thread = unsafe { OwnedHandle::from_raw_handle(process.hThread) };
    // SAFETY: all handles are live; the worker is still suspended and cannot execute content parsing.
    if let Err(error) = check(unsafe {
        AssignProcessToJobObject(job.as_raw_handle(), process_handle.as_raw_handle())
    }) {
        unsafe {
            TerminateProcess(process_handle.as_raw_handle(), 1);
            WaitForSingleObject(process_handle.as_raw_handle(), INFINITE);
        }
        return Err(error);
    }
    let process = Process {
        process: process_handle,
        job,
    };
    // SAFETY: owned primary thread of the suspended worker; process guard kills on failure.
    if unsafe { ResumeThread(thread.as_raw_handle()) } == u32::MAX {
        return Err(io::Error::last_os_error());
    }
    Ok(Worker {
        input,
        output,
        process,
    })
}
