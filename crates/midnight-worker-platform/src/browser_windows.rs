//! Trusted CEF broker in a kill-on-close job; CEF owns its nested renderer sandbox jobs.
use super::Browser;
pub use crate::platform::Process;
use crate::platform::{Attributes, check, pipe};
use std::{
    fs::OpenOptions,
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
    System::{JobObjects::*, Threading::*},
};

pub(super) fn profile_parent() -> io::Result<std::path::PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")
        .ok_or_else(|| io::Error::other("LOCALAPPDATA unavailable"))?;
    let base = std::path::PathBuf::from(base);
    if !base.is_absolute() {
        return Err(io::Error::other("Invalid LOCALAPPDATA"));
    }
    Ok(base.join("MidnightExplorer").join("web-viewer-sessions"))
}

pub(super) fn spawn(executable: &Path, profile: super::profile::Profile) -> io::Result<Browser> {
    let wide: Vec<u16> = executable.as_os_str().encode_wide().chain([0]).collect();
    if wide[..wide.len() - 1].contains(&0) {
        return Err(io::Error::other("NUL in browser path"));
    }
    let (child_input, input) = pipe()?;
    let (output, child_output) = pipe()?;
    let error = OpenOptions::new().write(true).open("NUL")?;
    // SAFETY: all three handles remain owned for CreateProcess; parent ends never inherit.
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
        "MIDNIGHT_WEB_VIEWER_PROFILE",
        "SystemRoot",
        "TEMP",
        "TMP",
    ] {
        let value = match key {
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
    // SAFETY: explicit image, no shell/arguments, restricted handle list, suspended before job.
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
    })?;
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
