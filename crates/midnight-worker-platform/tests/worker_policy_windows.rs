#![cfg(windows)]
use midnight_worker_platform::{CpuBudget, ResourcePolicy};
use std::{
    io::{Read, Write},
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    process::{Command, Stdio},
};
use windows_sys::Win32::System::JobObjects::*;

#[test]
fn no_additional_cpu_limit_keeps_inherited_job_policy() {
    const CHILD: &str = "MIDNIGHT_INHERITED_JOB_POLICY_TEST";
    if std::env::var_os(CHILD).is_some() {
        std::io::stdin().read_exact(&mut [0]).unwrap();
        let mut worker = midnight_worker_platform::spawn_with_policy(
            std::path::Path::new(env!("CARGO_BIN_EXE_midnight-worker-probe")),
            ResourcePolicy {
                memory_bytes: 512 * 1024 * 1024,
                cpu: CpuBudget::NoAdditionalLimit,
            },
        )
        .unwrap();
        midnight_worker_sdk::read_policy_ready(&mut worker.output).unwrap();
        worker.process.kill().unwrap();
        let _ = worker.process.wait();
        return;
    }
    // SAFETY: a new unnamed job; acquire ownership only after successful creation.
    let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    assert!(!job.is_null());
    let job = unsafe { OwnedHandle::from_raw_handle(job) };
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags =
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_PROCESS_TIME;
    limits.BasicLimitInformation.PerProcessUserTimeLimit = 60 * 10_000_000;
    // SAFETY: live test-owned job and correctly sized information. No breakaway is granted.
    assert_ne!(
        unsafe {
            SetInformationJobObject(
                job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of_val(&limits) as u32,
            )
        },
        0
    );
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "no_additional_cpu_limit_keeps_inherited_job_policy",
        ])
        .env(CHILD, "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // SAFETY: the owned test child waits on stdin before attempting its worker startup.
    assert_ne!(
        unsafe { AssignProcessToJobObject(job.as_raw_handle(), child.as_raw_handle()) },
        0
    );
    child.stdin.take().unwrap().write_all(b"g").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
