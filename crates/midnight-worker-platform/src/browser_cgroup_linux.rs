//! Private cgroup-v2 supervision for a trusted Chromium broker and its descendants.
//!
//! This bounds the whole process tree, including children which change process group. It does not
//! replace Chromium's namespace/seccomp sandbox. Only an existing user delegation is consumed;
//! no controller, limit or membership of an existing cgroup is changed.
use rustix::{
    fs::{self, AtFlags, Mode, OFlags},
    process::geteuid,
};
use std::{
    fs::File,
    io::{self, Read, Write},
    os::unix::fs::MetadataExt,
    path::{Component, Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime},
};

const CGROUP2_SUPER_MAGIC: u64 = 0x6367_7270;
const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NOFOLLOW);
const PARENT_ENV: &str = "MIDNIGHT_WEB_VIEWER_CGROUP_PARENT";
const CONTROL_LIMIT: u64 = 4096;
static NEXT: AtomicU64 = AtomicU64::new(1);

pub(super) struct Session {
    parent: File,
    directory: File,
    name: String,
    kill: File,
}

fn unavailable(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!(
            "Chromium requires a writable cgroup-v2 user delegation with memory and pids already \
             enabled. Launch Midnight Explorer within such a delegation or set {PARENT_ENV} to \
             its owned parent directory. No browser was started: {error}"
        ),
    )
}

impl Session {
    pub(super) fn new() -> io::Result<Self> {
        let parent = if let Some(path) = std::env::var_os(PARENT_ENV) {
            open_parent(Path::new(&path)).map_err(unavailable)?
        } else {
            discover_parent().map_err(unavailable)?
        };
        Self::create(parent).map_err(unavailable)
    }

    fn create(parent: File) -> io::Result<Self> {
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        let name = format!(
            "midnight-browser-{}-{stamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        // mkdirat is exclusive: a failed creation never owns or removes an existing cgroup.
        fs::mkdirat(&parent, &name, Mode::RUSR | Mode::WUSR | Mode::XUSR)?;
        let directory = match fs::openat(&parent, &name, DIRECTORY_FLAGS, Mode::empty()) {
            Ok(directory) => File::from(directory),
            Err(error) => {
                let _ = fs::unlinkat(&parent, &name, AtFlags::REMOVEDIR);
                return Err(error.into());
            }
        };
        let prepared = (|| {
            let metadata = directory.metadata()?;
            if metadata.uid() != geteuid().as_raw() || metadata.mode() & 0o077 != 0 {
                return Err(io::Error::other("New browser cgroup is not private"));
            }
            configure_limits(&directory)?;
            // Require tree-wide cancellation support before any executable starts (Linux 5.14+).
            open_control(&directory, "cgroup.kill", OFlags::WRONLY)
        })();
        match prepared {
            Ok(kill) => Ok(Self {
                parent,
                directory,
                name,
                kill,
            }),
            Err(error) => {
                remove_owned(&parent, &directory, &name);
                Err(error)
            }
        }
    }

    pub(super) fn prepare_command(&self, command: &mut Command) -> io::Result<()> {
        use std::os::unix::process::CommandExt;
        let membership = open_control(&self.directory, "cgroup.procs", OFlags::WRONLY)?;
        // SAFETY: after fork this closure only performs write(2), retries EINTR and returns an
        // allocation-free errno/kind. The descriptor is preopened and CLOEXEC. Writing zero moves
        // the calling child, before exec or any Chromium thread/fork, into the bounded cgroup.
        // Chromium's own sandbox setup is left intact; no decoder seccomp policy is installed.
        unsafe {
            command.pre_exec(move || {
                loop {
                    match rustix::io::write(&membership, b"0\n") {
                        Ok(2) => return Ok(()),
                        Ok(_) => return Err(io::ErrorKind::WriteZero.into()),
                        Err(rustix::io::Errno::INTR) => continue,
                        Err(error) => return Err(error.into()),
                    }
                }
            });
        }
        Ok(())
    }

    pub(super) fn kill(&mut self) -> io::Result<()> {
        write_value(&mut self.kill, b"1\n")
    }

    pub(super) fn wait_empty(&self) -> io::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let events = read_control(&self.directory, "cgroup.events")?;
            if events.lines().any(|line| line == "populated 0") {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Chromium cgroup still contains processes after tree termination",
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.kill();
        if self.wait_empty().is_ok() {
            remove_owned(&self.parent, &self.directory, &self.name);
        }
    }
}

fn remove_owned(parent: &File, directory: &File, name: &str) {
    if let (Ok(owned), Ok(named)) = (
        directory.metadata(),
        fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW),
    ) && owned.dev() == named.st_dev
        && owned.ino() == named.st_ino
    {
        let _ = fs::unlinkat(parent, name, AtFlags::REMOVEDIR);
    }
}

fn configure_limits(directory: &File) -> io::Result<()> {
    // Linux pids counts threads, unlike the Windows job's 32-process budget. Both platforms use
    // 2 GiB aggregate memory; disable swap too so the cgroup cannot exhaust host swap separately.
    for (name, value) in [
        ("memory.max", "2147483648"),
        ("memory.swap.max", "0"),
        ("memory.oom.group", "1"),
        ("pids.max", "256"),
    ] {
        let mut control = open_control(directory, name, OFlags::WRONLY)?;
        write_value(&mut control, value.as_bytes())?;
        if read_control(directory, name)?.trim() != value {
            return Err(io::Error::other(format!(
                "Browser cgroup rejected {name} limit"
            )));
        }
    }
    Ok(())
}

fn write_value(file: &mut File, value: &[u8]) -> io::Result<()> {
    // cgroup controls accept one complete value in a single write. Never retry a partial value.
    loop {
        match file.write(value) {
            Ok(written) if written == value.len() => return Ok(()),
            Ok(_) => return Err(io::ErrorKind::WriteZero.into()),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
}

fn open_control(directory: &File, name: &str, access: OFlags) -> io::Result<File> {
    let control = File::from(fs::openat(
        directory,
        name,
        access | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::NOCTTY,
        Mode::empty(),
    )?);
    let metadata = control.metadata()?;
    if !metadata.is_file() || metadata.uid() != geteuid().as_raw() || metadata.mode() & 0o022 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Browser cgroup control must be a regular owned file without foreign write access",
        ));
    }
    Ok(control)
}

fn read_control(directory: &File, name: &str) -> io::Result<String> {
    let file = open_control(directory, name, OFlags::RDONLY)?;
    read_bounded(file, CONTROL_LIMIT)
}

fn read_bounded(file: File, limit: u64) -> io::Result<String> {
    let mut value = String::new();
    file.take(limit + 1).read_to_string(&mut value)?;
    if value.len() as u64 > limit {
        return Err(io::Error::other(
            "Browser cgroup metadata exceeds its bound",
        ));
    }
    Ok(value)
}

fn open_parent(path: &Path) -> io::Result<File> {
    if !path.is_absolute() {
        return Err(io::Error::other("Delegated cgroup parent must be absolute"));
    }
    let uid = geteuid().as_raw();
    let mut directory = File::from(fs::open("/", DIRECTORY_FLAGS, Mode::empty())?);
    for component in path.components() {
        let Component::Normal(name) = component else {
            if component == Component::RootDir {
                continue;
            }
            return Err(io::Error::other(
                "Delegated cgroup parent contains traversal",
            ));
        };
        directory = File::from(fs::openat(
            &directory,
            name,
            DIRECTORY_FLAGS,
            Mode::empty(),
        )?);
        let metadata = directory.metadata()?;
        // A root/owned sticky directory prevents other users replacing our entries too.
        if ![0, uid].contains(&metadata.uid())
            || (metadata.mode() & 0o022 != 0 && metadata.mode() & 0o1000 == 0)
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Delegated cgroup ancestry permits foreign replacement",
            ));
        }
    }
    let metadata = directory.metadata()?;
    if metadata.uid() != uid
        || metadata.mode() & 0o022 != 0
        || fs::fstatfs(&directory)?.f_type as u64 != CGROUP2_SUPER_MAGIC
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Delegated parent must be an owned native cgroup-v2 directory",
        ));
    }
    let controllers = read_control(&directory, "cgroup.subtree_control")?;
    if !["memory", "pids"].iter().all(|required| {
        controllers
            .split_whitespace()
            .any(|value| value == *required)
    }) {
        return Err(io::Error::other(
            "Delegated parent must already enable memory and pids",
        ));
    }
    // Opening does not migrate or alter the application's cgroup. Kernel migration permission
    // checks on the common ancestor still apply to the child's eventual pre-exec write.
    open_control(&directory, "cgroup.procs", OFlags::WRONLY)?;
    Ok(directory)
}

fn discover_parent() -> io::Result<File> {
    let membership = read_bounded(File::open("/proc/self/cgroup")?, 64 * 1024)?;
    let unified = membership
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .ok_or_else(|| io::Error::other("No unified cgroup-v2 membership"))?;
    if !Path::new(unified).is_absolute()
        || Path::new(unified)
            .components()
            .any(|value| !matches!(value, Component::Normal(_) | Component::RootDir))
    {
        return Err(io::Error::other("Invalid unified cgroup-v2 membership"));
    }
    let root = Path::new("/sys/fs/cgroup");
    let mut path = PathBuf::from(root);
    path.extend(
        Path::new(unified)
            .components()
            .filter_map(|value| match value {
                Component::Normal(name) => Some(name),
                _ => None,
            }),
    );
    while path.starts_with(root) {
        if let Ok(parent) = open_parent(&path) {
            return Ok(parent);
        }
        if path == root || !path.pop() {
            break;
        }
    }
    Err(io::Error::other(
        "No existing owned memory/pids delegation found",
    ))
}

#[cfg(test)]
#[path = "browser_cgroup_linux_tests.rs"]
mod tests;
