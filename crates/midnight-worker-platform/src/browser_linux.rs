//! Trusted broker in a bounded cgroup. Chromium retains its namespace/seccomp renderer sandbox.
use super::Browser;
#[path = "browser_cgroup_linux.rs"]
mod cgroup;
#[path = "browser_libraries_linux.rs"]
mod libraries;
#[path = "browser_sandbox_linux.rs"]
mod sandbox;
#[path = "browser_temporary_linux.rs"]
mod temporary;
#[cfg(test)]
#[path = "browser_linux_tests.rs"]
mod tests;
use std::{
    fs::File,
    io,
    os::{fd::OwnedFd, unix::process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

pub struct Process {
    child: Child,
    cgroup: cgroup::Session,
    temporary: temporary::Directory,
    executable: PathBuf,
}
impl Process {
    pub fn kill(&mut self) -> io::Result<()> {
        self.cgroup.kill()
    }
    pub fn wait(&mut self) -> io::Result<bool> {
        let status = self.child.wait()?;
        // A broker may exit before descendants, including children with independent groups.
        self.cgroup.kill()?;
        self.cgroup.wait_empty()?;
        self.temporary.tree_reaped();
        Ok(status.success())
    }
    /// Why a browser that ended before its first message could not start, once `wait`
    /// returned: when the dynamic loader stopped it (exit status 127), the shared libraries
    /// that the system lacks, as a [`LibrariesMissing`](super::LibrariesMissing) that names
    /// them and how to install them. `None` for any other end, and while the browser runs.
    /// Reads the executable's and its libraries' dynamic sections; runs nothing.
    pub fn startup_failure(&mut self) -> Option<io::Error> {
        let status = self.child.try_wait().ok().flatten()?;
        libraries::diagnose(status, &self.executable)
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.kill();
        let _ = self.wait();
    }
}
/// Whether a session's recording process may still run. Only a definitely missing process
/// counts as ended; a process owned by another user or any other failure keeps its session.
pub(super) fn process_alive(pid: u32) -> bool {
    let Some(pid) = i32::try_from(pid)
        .ok()
        .and_then(rustix::process::Pid::from_raw)
    else {
        return true;
    };
    rustix::process::test_kill_process(pid) != Err(rustix::io::Errno::SRCH)
}
pub(super) fn profile_parent() -> io::Result<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|value| std::path::PathBuf::from(value).join(".cache"))
        })
        .ok_or_else(|| io::Error::other("Private cache directory unavailable"))?;
    if !base.is_absolute() {
        return Err(io::Error::other("Invalid cache directory"));
    }
    Ok(base.join("midnight-explorer").join("web-viewer-sessions"))
}
pub(super) fn spawn(
    executable: &Path,
    profile: super::profile::Profile,
    host_shortcuts: bool,
) -> io::Result<Browser> {
    let namespaces = sandbox::namespaces(executable);
    let cgroup = cgroup::Session::new()?;
    let mut temporary = temporary::Directory::new()?;
    let mut command = Command::new(executable);
    command
        .env_clear()
        .current_dir("/")
        .process_group(0)
        .env("MIDNIGHT_WEB_VIEWER_PROFILE", profile.path())
        .env("TMPDIR", temporary.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if host_shortcuts {
        command.env(super::HOST_SHORTCUTS_ENV, super::HOST_SHORTCUTS_PHYSICAL_V1);
    }
    for key in [
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XDG_RUNTIME_DIR",
        "XAUTHORITY",
        "LANG",
        "LC_ALL",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    cgroup.prepare_command(&mut command)?;
    let child = command.spawn().map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("Cannot start bounded Chromium broker: {error}"),
        )
    })?;
    temporary.retain_until_reaped();
    let mut process = Process {
        child,
        cgroup,
        temporary,
        executable: executable.to_path_buf(),
    };
    // Checked as soon as Chromium has executed, when its AppArmor label is final and long
    // before it tries its sandbox. Dropping the process stops and reaps its tree.
    if let Some(refused) = sandbox::refusal(process.child.id(), namespaces, executable) {
        drop(process);
        return Err(refused);
    }
    let input = process
        .child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("Missing browser input"))?;
    let output = process
        .child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("Missing browser output"))?;
    Ok(Browser {
        input: File::from(OwnedFd::from(input)),
        output: File::from(OwnedFd::from(output)),
        process,
        _profile: profile,
    })
}
