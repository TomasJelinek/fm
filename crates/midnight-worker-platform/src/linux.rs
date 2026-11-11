use crate::{ResourcePolicy, Worker};
use std::{
    fs::File,
    io,
    os::fd::OwnedFd,
    path::Path,
    process::{Child, Command, Stdio},
};

pub struct Process(Child);
impl Process {
    pub fn kill(&mut self) -> io::Result<()> {
        self.0.kill()
    }
    pub fn wait(&mut self) -> io::Result<bool> {
        self.0.wait().map(|status| status.success())
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
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
