//! Host-owned process boundary shared by media and archive executable plugins.
#![deny(unsafe_op_in_unsafe_fn)]

pub use midnight_worker_sdk::{CpuBudget, Limits, ResourcePolicy};
use std::{
    collections::HashSet,
    fmt,
    fs::File,
    io,
    path::{Path, PathBuf},
    sync::Mutex,
};

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

    /// Why the process ended, for a message that people read, with the native status in
    /// parentheses: "a required library is missing or cannot be read (STATUS_DLL_NOT_FOUND,
    /// 0xC0000135)" for a Windows worker whose loader could not open a library, "it crashed
    /// (SIGSEGV)" for a Linux one, or "it exited with code …" for any other status.
    pub fn reason(self) -> String {
        platform::exit_reason(self)
    }

    /// The worker's loader could not load a library: `STATUS_DLL_NOT_FOUND` on Windows,
    /// which the loader also reports for a library it was denied, or exit code 127 from
    /// the dynamic loader on Linux. [`unloadable_library`] then finds which one.
    pub fn library_not_loaded(self) -> bool {
        platform::library_not_loaded(self)
    }
}

/// A library that a worker's loader cannot load: missing, or present but not readable by
/// the worker (on Windows, by the workers' AppContainer).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnloadableLibrary {
    pub path: PathBuf,
    pub missing: bool,
}

impl fmt::Display for UnloadableLibrary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Installation paths are verbatim on Windows; people read and type them without
        // the `\\?\` namespace prefix.
        let path = self.path.display().to_string();
        let path = match path.strip_prefix(r"\\?\") {
            Some(unc) if unc.starts_with(r"UNC\") => format!(r"\\{}", &unc[4..]),
            Some(local) => local.to_owned(),
            None => path,
        };
        if self.missing {
            write!(formatter, "could not load its libraries: {path} is missing")
        } else {
            write!(
                formatter,
                "could not load its libraries (access denied): {path}{}",
                platform::DENIED_GUIDANCE
            )
        }
    }
}

/// After a worker ended with [`ExitStatus::library_not_loaded`]: the first of `libraries`
/// that is missing, or that the workers may not open as their loader does (read and execute;
/// on Windows evaluated for the workers' AppContainer from the file's security descriptor).
/// `None` when every one is present and loadable, or when that cannot be determined.
/// Call on a backend worker: this reads each file's metadata and security.
pub fn unloadable_library(libraries: &[PathBuf]) -> Option<UnloadableLibrary> {
    libraries.iter().find_map(|path| {
        let missing = match std::fs::metadata(path) {
            Ok(_) => false,
            Err(error) if error.kind() == io::ErrorKind::NotFound => true,
            Err(_) => return None,
        };
        (missing || matches!(platform::worker_may_load(path), Ok(false))).then(|| {
            UnloadableLibrary {
                path: path.clone(),
                missing,
            }
        })
    })
}

/// What [`grant_worker_load`] found or did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadGrant {
    /// The workers could already load the file; nothing was changed.
    Present,
    /// The file's own permissions now also let the workers' AppContainer read and execute
    /// it (Windows).
    Added,
    /// The workers cannot load the file, and `may_change` refused changing it.
    Refused,
}

/// Let the restricted workers load `path` as their loader opens a library (read and
/// execute). When they already may (the check [`unloadable_library`] makes), nothing
/// changes. Otherwise, only when `may_change(path)` allows it, Windows adds one explicit
/// entry without inheritance flags to the file's DACL, merged with the entries it has, that
/// allows read and execute (0x1200A9) to the workers' AppContainer
/// (`MidnightExplorer.RestrictedWorker.v1`) and to no one else; folders are never changed.
/// It fails when the user may not change the file's permissions, or when the workers still
/// may not load it afterwards (an entry that denies them). Linux workers read files as the
/// user, so there is nothing to grant. Call on a backend worker.
pub fn grant_worker_load(
    path: &Path,
    may_change: impl FnOnce(&Path) -> bool,
) -> io::Result<LoadGrant> {
    if platform::worker_may_load(path)? {
        return Ok(LoadGrant::Present);
    }
    if !may_change(path) {
        return Ok(LoadGrant::Refused);
    }
    platform::grant_load(path)?;
    if !platform::worker_may_load(path)? {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "{} still denies the workers after the grant",
                path.display()
            ),
        ));
    }
    Ok(LoadGrant::Added)
}

/// The workers and files that [`prepare_worker`] found loadable in this run.
#[derive(Default)]
struct Prepared {
    workers: HashSet<PathBuf>,
    files: HashSet<PathBuf>,
}

static PREPARED: Mutex<Option<Prepared>> = Mutex::new(None);

/// Before starting the restricted worker `executable`: [`grant_worker_load`] for each of
/// `files`, the installed files its package lets the workers load. Outcomes are kept for the
/// run: a worker whose files are all loadable, and each loadable file, are not checked again,
/// so later launches only look them up. A refused or failed file is checked again at the
/// next launch, so that securing the installation takes effect without a restart. Launches
/// are serialized while files are checked or changed; `files` is called only then. Returns
/// the first failure; the worker may still start (a later diagnosis names a library it
/// cannot load). Call on a backend worker.
pub fn prepare_worker(
    executable: &Path,
    files: impl FnOnce() -> Vec<PathBuf>,
    may_change: impl Fn(&Path) -> bool,
) -> io::Result<()> {
    let mut prepared = PREPARED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let prepared = prepared.get_or_insert_with(Prepared::default);
    if prepared.workers.contains(executable) {
        return Ok(());
    }
    let mut result = Ok(());
    for file in files() {
        if prepared.files.contains(&file) {
            continue;
        }
        match grant_worker_load(&file, &may_change) {
            Ok(LoadGrant::Present | LoadGrant::Added) => {
                prepared.files.insert(file);
            }
            Ok(LoadGrant::Refused) => {
                if result.is_ok() {
                    result = Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        format!(
                            "{} was left unchanged: its installation is not private",
                            file.display()
                        ),
                    ));
                }
            }
            Err(error) => {
                if result.is_ok() {
                    result = Err(error);
                }
            }
        }
    }
    if result.is_ok() {
        prepared.workers.insert(executable.to_owned());
    }
    result
}

/// The libraries in the folder of a worker `executable`, where its loader looks first:
/// `*.dll` on Windows, `*.so` and `*.so.*` on Linux, in name order. Call on a backend worker.
pub fn worker_libraries(executable: &Path) -> Vec<PathBuf> {
    let Some(Ok(entries)) = executable.parent().map(std::fs::read_dir) else {
        return Vec::new();
    };
    let mut libraries: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(platform::library_name)
        })
        .collect();
    libraries.sort();
    libraries
}

/// A worker that ended before the host could send it its startup message, for example
/// because a library it needs is missing. Hosts word their own message around [`reason`].
///
/// [`reason`]: ExitStatus::reason
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkerStopped(pub ExitStatus);

impl fmt::Display for WorkerStopped {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "The worker stopped: {}", self.0.reason())
    }
}

impl std::error::Error for WorkerStopped {}

impl Worker {
    /// The startup message could not be written. A worker that has already ended closed its
    /// input: its exit status, in a [`WorkerStopped`] of the same error kind, says why. The
    /// kill only bounds the wait; it does not replace the status of a process that has
    /// already exited. Any other failure is returned unchanged.
    fn explain_startup_failure(mut self, error: io::Error) -> io::Error {
        if error.kind() != io::ErrorKind::BrokenPipe {
            return error;
        }
        let _ = self.process.kill();
        match self.process.wait_status() {
            Ok(status) => io::Error::new(error.kind(), WorkerStopped(status)),
            Err(_) => error,
        }
    }
}

/// Launch only a trusted, absolute installed executable. No media path, shell, environment secrets,
/// or inherited source handle is supplied. The caller must verify `read_ready` with a deadline.
/// A worker that cannot start never shows a system dialog; see [`WorkerStopped`].
pub fn spawn(executable: &Path, limits: Limits) -> io::Result<Worker> {
    validate_path(executable)?;
    let limits = limits.validate()?;
    let mut worker = platform::spawn(executable, limits.into())?;
    if let Err(error) = midnight_worker_sdk::write_limits(&mut worker.input, limits) {
        return Err(worker.explain_startup_failure(error));
    }
    Ok(worker)
}

/// Explicit version2 resource policy. Verify `read_policy_ready` with a host deadline.
/// NoAdditionalLimit does not remove memory/process isolation or cancellation.
/// Inherited OS resource and security policies remain in force.
pub fn spawn_with_policy(executable: &Path, policy: ResourcePolicy) -> io::Result<Worker> {
    validate_path(executable)?;
    let policy = policy.validate()?;
    let mut worker = platform::spawn(executable, policy)?;
    if let Err(error) = midnight_worker_sdk::write_policy(&mut worker.input, policy) {
        return Err(worker.explain_startup_failure(error));
    }
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
    use std::path::{Path, PathBuf};

    #[test]
    fn an_unloadable_library_is_named_as_people_type_its_path() {
        for (path, shown) in [
            (
                r"\\?\C:\Apps\thumbnailers\avcodec-63.dll",
                r"C:\Apps\thumbnailers\avcodec-63.dll",
            ),
            (
                r"\\?\UNC\server\share\avcodec-63.dll",
                r"\\server\share\avcodec-63.dll",
            ),
            ("/opt/app/libavcodec.so.63", "/opt/app/libavcodec.so.63"),
        ] {
            let mut library = super::UnloadableLibrary {
                path: PathBuf::from(path),
                missing: false,
            };
            assert_eq!(
                library.to_string(),
                format!(
                    "could not load its libraries (access denied): {shown}{}",
                    super::platform::DENIED_GUIDANCE
                )
            );
            library.missing = true;
            assert_eq!(
                library.to_string(),
                format!("could not load its libraries: {shown} is missing")
            );
        }
    }

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
