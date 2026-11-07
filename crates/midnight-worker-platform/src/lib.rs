//! Host-owned process boundary shared by media and archive executable plugins.
#![deny(unsafe_op_in_unsafe_fn)]

pub use midnight_worker_sdk::Limits;
use std::{fs::File, io, path::Path};

#[cfg(windows)]
#[path = "windows.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod platform;
pub use platform::Process;
pub mod store;

pub struct Worker {
    pub input: File,
    pub output: File,
    pub process: Process,
}

/// Launch only a trusted, absolute installed executable. No media path, shell, environment secrets,
/// or inherited source handle is supplied. The caller must verify `read_ready` with a deadline.
pub fn spawn(executable: &Path, limits: Limits) -> io::Result<Worker> {
    if !executable.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Worker path must be absolute",
        ));
    }
    let limits = limits.validate()?;
    let mut worker = platform::spawn(executable, limits)?;
    midnight_worker_sdk::write_limits(&mut worker.input, limits)?;
    Ok(worker)
}
