//! Host-owned process boundary shared by media and archive executable plugins.
#![deny(unsafe_op_in_unsafe_fn)]

pub use midnight_worker_sdk::{CpuBudget, Limits, ResourcePolicy};
use std::{fs::File, io, path::Path};

#[cfg(windows)]
#[path = "windows.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod platform;
pub use platform::Process;
pub mod browser;
pub mod spool;
pub mod store;

pub struct Worker {
    pub input: File,
    pub output: File,
    pub process: Process,
}

/// Launch only a trusted, absolute installed executable. No media path, shell, environment secrets,
/// or inherited source handle is supplied. The caller must verify `read_ready` with a deadline.
pub fn spawn(executable: &Path, limits: Limits) -> io::Result<Worker> {
    validate_path(executable)?;
    let limits = limits.validate()?;
    let mut worker = platform::spawn(executable, limits.into())?;
    midnight_worker_sdk::write_limits(&mut worker.input, limits)?;
    Ok(worker)
}

/// Explicit version2 resource policy. Verify `read_policy_ready` with a host deadline.
/// NoAdditionalLimit does not remove memory/process isolation or cancellation.
/// Inherited OS resource and security policies remain in force.
pub fn spawn_with_policy(executable: &Path, policy: ResourcePolicy) -> io::Result<Worker> {
    validate_path(executable)?;
    let policy = policy.validate()?;
    let mut worker = platform::spawn(executable, policy)?;
    midnight_worker_sdk::write_policy(&mut worker.input, policy)?;
    Ok(worker)
}

fn validate_path(executable: &Path) -> io::Result<()> {
    if !executable.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Worker path must be absolute",
        ));
    }
    Ok(())
}
