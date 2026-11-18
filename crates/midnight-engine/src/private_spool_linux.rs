//! Linux spool bases: the XDG base directories, then the temporary directory
//! (usually the shared `/tmp`) under this account's effective user ID.
use super::Session;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

#[cfg(test)]
#[path = "private_spool_linux_tests.rs"]
mod tests;

pub(super) fn current() -> Session {
    from_environment(
        |name| std::env::var_os(name),
        |path| std::fs::canonicalize(path).ok(),
        effective_uid(),
        std::env::temp_dir(),
    )
}

/// A relative or empty XDG variable is ignored, as the XDG base directory
/// specification requires; the cache then defaults to `~/.cache`. Links in the
/// cache and runtime bases are resolved (`resolve`), so a linked home such as
/// `/home` -> `/var/home` keeps its disk-backed cache root. The cache root still
/// fails on an NFS home without `O_TMPFILE`, and sessions without
/// `XDG_RUNTIME_DIR` (su, sudo, ssh or containers) have no runtime root, so
/// those reach the temporary directory.
fn from_environment(
    variable: impl Fn(&str) -> Option<OsString>,
    canonical: impl Fn(&Path) -> Option<PathBuf>,
    uid: Option<u32>,
    temporary: PathBuf,
) -> Session {
    let absolute = |name: &str| {
        variable(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    let cache =
        absolute("XDG_CACHE_HOME").or_else(|| absolute("HOME").map(|home| home.join(".cache")));
    Session {
        application: "midnight-explorer",
        cache: cache.map(|base| resolve(&base, &canonical)),
        runtime: absolute("XDG_RUNTIME_DIR").map(|base| resolve(&base, &canonical)),
        // The private-root check compares the owner with the effective user ID,
        // which `USER` need not name (`su` keeps it), so that ID is the label.
        account: uid.map(|uid| uid.to_string()),
        temporary,
    }
}

/// Resolve links in a base directory that the user or the system chose, such as
/// `/home` linked to `/var/home` or a linked `~/.cache`, including trailing
/// components that do not exist yet; the filesystem host resolves its staging
/// bases the same way. The private-root walk then follows no link: the
/// `midnight-explorer/<name>` directories below a base are never resolved, so a
/// link at or below them is still refused. A base that cannot be resolved is used
/// as entered.
fn resolve(base: &Path, canonical: &impl Fn(&Path) -> Option<PathBuf>) -> PathBuf {
    let mut missing = Vec::new();
    let mut existing = base;
    loop {
        if let Some(resolved) = canonical(existing) {
            return missing
                .iter()
                .rev()
                .fold(resolved, |path, name| path.join(name));
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                missing.push(name);
                existing = parent;
            }
            _ => return base.to_owned(),
        }
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
