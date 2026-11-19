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
/// Hold while spawning a child that inherits every inheritable handle (Windows only), and only
/// for the creation call itself; [`spawn_command`] does both for `std::process::Command`.
#[cfg(windows)]
pub use platform::lock_process_creation;
pub mod browser;
pub mod spool;
pub mod store;

#[cfg(feature = "startup-diagnostics")]
#[doc(hidden)]
pub mod startup_diagnostics;

pub struct Worker {
    pub input: File,
    pub output: File,
    pub process: Process,
}

/// How a worker process ended, as the OS reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitStatus {
    /// The exit code: all 32 bits on Windows (`0xC0000005` after an access violation,
    /// 1 after [`Process::kill`]), 0 to 255 on Linux.
    Code(u32),
    /// Linux: the signal that terminated the process (9 after [`Process::kill`]).
    Signal(i32),
}
impl ExitStatus {
    pub fn success(self) -> bool {
        self == Self::Code(0)
    }
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

/// Spawn an unrelated child process (an editor, a custom command, a restarted app) from a
/// process that may also be starting workers. On Windows `std::process::Command` lets the
/// child inherit every inheritable handle, so the spawn waits for any worker or browser
/// creation that briefly holds inheritable pipe ends. The lock covers only the `spawn` call,
/// never the child's lifetime: wait for the child or read its output after this returns.
pub fn spawn_command(command: &mut std::process::Command) -> io::Result<std::process::Child> {
    #[cfg(windows)]
    let _creation = platform::lock_process_creation();
    command.spawn()
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    /// Cargo.toml sets `test = false` for the `midnight-worker-probe` binary, so
    /// `cargo test` builds no harness for it and a unit test there would never run.
    #[test]
    fn probe_binary_has_no_unit_tests() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/midnight-worker-probe.rs");
        let source = std::fs::read_to_string(&path).expect("binary source");
        for line in source.lines().map(str::trim) {
            let words: Vec<_> = line
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .collect();
            let attribute = line.starts_with("#[") || line.starts_with("#![");
            // A module file would hold code that this check does not read.
            let module_file =
                !line.starts_with("//") && words.contains(&"mod") && line.ends_with(';');
            assert!(
                !(attribute && words.contains(&"test")) && !module_file,
                "{}: `{line}`; remove `test = false` from the binary in Cargo.toml",
                path.display()
            );
        }
    }
}
