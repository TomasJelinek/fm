use crate::Limits;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use windows_sys::Win32::{
    Security::*,
    System::{JobObjects::*, Threading::*},
};

pub(super) fn restrict(limits: Limits) -> io::Result<()> {
    let mut token = std::ptr::null_mut();
    // SAFETY: fixed current-process pseudo handle and valid token output; own only on success.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut app_container = 0u32;
    let mut returned = 0;
    let mut budget = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    // SAFETY: token is live; buffers exactly match documented token/job query types.
    unsafe {
        if GetTokenInformation(
            token.as_raw_handle(),
            TokenIsAppContainer,
            (&mut app_container as *mut u32).cast(),
            4,
            &mut returned,
        ) == 0
            || QueryInformationJobObject(
                std::ptr::null_mut(),
                JobObjectExtendedLimitInformation,
                (&mut budget as *mut JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of_val(&budget) as u32,
                std::ptr::null_mut(),
            ) == 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    let required = JOB_OBJECT_LIMIT_ACTIVE_PROCESS
        | JOB_OBJECT_LIMIT_PROCESS_MEMORY
        | JOB_OBJECT_LIMIT_PROCESS_TIME;
    if app_container != 1
        || budget.BasicLimitInformation.LimitFlags & required != required
        || budget.BasicLimitInformation.ActiveProcessLimit != 1
        || budget.ProcessMemoryLimit as u64 > limits.memory_bytes
        || budget.BasicLimitInformation.PerProcessUserTimeLimit
            > i64::from(limits.cpu_seconds) * 10_000_000
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Required worker containment is absent",
        ));
    }
    Ok(())
}
