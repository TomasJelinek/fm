//! Linux spool bases: the XDG base directories, then the temporary directory
//! (usually the shared `/tmp`) under this account's effective user ID.
use super::Session;
use std::{ffi::OsString, path::PathBuf};

#[cfg(test)]
#[path = "private_spool_linux_tests.rs"]
mod tests;

pub(super) fn current() -> Session {
    from_environment(
        |name| std::env::var_os(name),
        effective_uid(),
        std::env::temp_dir(),
    )
}

/// A relative or empty XDG variable is ignored, as the XDG base directory
/// specification requires; the cache then defaults to `~/.cache`. The cache root
/// fails on an NFS home without `O_TMPFILE` and below a symlinked home component,
/// and sessions without `XDG_RUNTIME_DIR` (su, sudo, ssh or containers) have no
/// runtime root, so those reach the temporary directory.
fn from_environment(
    variable: impl Fn(&str) -> Option<OsString>,
    uid: Option<u32>,
    temporary: PathBuf,
) -> Session {
    let absolute = |name: &str| {
        variable(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    Session {
        application: "midnight-explorer",
        cache: absolute("XDG_CACHE_HOME")
            .or_else(|| absolute("HOME").map(|home| home.join(".cache"))),
        runtime: absolute("XDG_RUNTIME_DIR"),
        // The private-root check compares the owner with the effective user ID,
        // which `USER` need not name (`su` keeps it), so that ID is the label.
        account: uid.map(|uid| uid.to_string()),
        temporary,
    }
}

/// The effective user ID: `Uid:` lists the real, effective, saved and
/// filesystem user IDs. None only omits the per-user temporary name.
fn effective_uid() -> Option<u32> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}
