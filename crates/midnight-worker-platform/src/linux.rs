use crate::{ExitStatus, ResourcePolicy, Worker};
use std::{
    fs::File,
    io,
    os::{fd::OwnedFd, unix::process::ExitStatusExt},
    path::Path,
    process::{Child, Command, Stdio},
};

pub struct Process(Child);
impl Process {
    /// Confirm the child was reaped even when shutdown intentionally produces a nonzero
    /// status. wait() retains its existing success-status result for ordinary callers.
    pub fn wait_terminated(&mut self) -> io::Result<()> {
        self.0.wait().map(|_| ())
    }
    pub fn kill(&mut self) -> io::Result<()> {
        self.0.kill()
    }
    pub fn wait(&mut self) -> io::Result<bool> {
        self.wait_status().map(ExitStatus::success)
    }
    /// Wait for the worker to exit and return its exit code or terminating signal.
    pub fn wait_status(&mut self) -> io::Result<ExitStatus> {
        let status = self.0.wait()?;
        match (status.code(), status.signal()) {
            (Some(code), _) => Ok(ExitStatus::Code(code.cast_unsigned())),
            (None, Some(signal)) => Ok(ExitStatus::Signal(signal)),
            // A reaped child either exited or was signalled; wait() never reports a stop.
            (None, None) => Err(io::Error::other(format!(
                "Worker wait returned an unexpected status: {status}"
            ))),
        }
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// [`ExitStatus::reason`] on Linux: the dynamic loader's status for a library it could not
/// load and the signals of a crash, an abort or a kill, in words; anything else as is.
pub(super) fn exit_reason(status: ExitStatus) -> String {
    match status {
        // ld.so exits with 127 when a needed shared library is missing or unreadable.
        ExitStatus::Code(127) => {
            "a required library is missing or cannot be read (exit code 127)".into()
        }
        ExitStatus::Code(code) => format!("it exited with code {code}"),
        ExitStatus::Signal(signal) => {
            let known = match signal {
                4 => (
                    "it crashed on an unsupported processor instruction",
                    "SIGILL",
                ),
                6 => ("it stopped on an internal error", "SIGABRT"),
                7 => ("it crashed", "SIGBUS"),
                8 => ("it crashed on an arithmetic error", "SIGFPE"),
                9 => ("it was stopped", "SIGKILL"),
                11 => ("it crashed", "SIGSEGV"),
                24 => ("it used up its processor time", "SIGXCPU"),
                31 => ("it made a call that its sandbox refuses", "SIGSYS"),
                _ => return format!("it was terminated by signal {signal}"),
            };
            format!("{} ({})", known.0, known.1)
        }
    }
}

/// ld.so's exit code when it cannot load a needed shared library.
pub(super) fn library_not_loaded(status: ExitStatus) -> bool {
    status == ExitStatus::Code(127)
}

pub(super) fn library_name(name: &str) -> bool {
    name.ends_with(".so") || name.contains(".so.")
}

/// Whether a worker, which runs as this user, may open `path`: a library it cannot read
/// fails to load like a missing one.
pub(super) fn worker_may_load(path: &Path) -> io::Result<bool> {
    match File::open(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => Ok(false),
        Err(error) => Err(error),
    }
}

pub(super) fn spawn(executable: &Path, _: ResourcePolicy) -> io::Result<Worker> {
    #[cfg(feature = "startup-diagnostics")]
    let mut startup = crate::startup_diagnostics::Trace::new("restricted_linux");
    // The trusted SDK locks down before acknowledging startup or accepting any untrusted bytes.
    let mut child = Command::new(executable)
        .env_clear()
        .current_dir("/")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    #[cfg(feature = "startup-diagnostics")]
    startup.mark("process_created");
    let input = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("Missing worker input"))?;
    let output = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("Missing worker output"))?;
    Ok(Worker {
        input: File::from(OwnedFd::from(input)),
        output: File::from(OwnedFd::from(output)),
        process: Process(child),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_reasons_name_the_loader_crash_and_sandbox_statuses() {
        for (status, reason) in [
            (
                ExitStatus::Code(127),
                "a required library is missing or cannot be read (exit code 127)",
            ),
            (ExitStatus::Code(101), "it exited with code 101"),
            (ExitStatus::Signal(11), "it crashed (SIGSEGV)"),
            (
                ExitStatus::Signal(6),
                "it stopped on an internal error (SIGABRT)",
            ),
            (ExitStatus::Signal(9), "it was stopped (SIGKILL)"),
            (
                ExitStatus::Signal(31),
                "it made a call that its sandbox refuses (SIGSYS)",
            ),
            (ExitStatus::Signal(15), "it was terminated by signal 15"),
        ] {
            assert_eq!(status.reason(), reason);
        }
    }
}
