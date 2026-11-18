//! Linux spool bases: the XDG base directories, then the temporary directory
//! (usually the shared `/tmp`) under this account's effective user ID.
use super::{Session, SpoolContent};
use midnight_worker_platform::spool::{effective_uid, resolve_base};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

#[cfg(test)]
#[path = "private_spool_linux_tests.rs"]
mod tests;

pub(super) fn current(content: SpoolContent) -> Session {
    from_environment(
        |name| std::env::var_os(name),
        |path| std::fs::canonicalize(path).ok(),
        effective_uid(),
        std::env::temp_dir(),
        content,
    )
}

/// A relative or empty XDG variable is ignored, as the XDG base directory
/// specification requires; the cache then defaults to `~/.cache`. Links in the
/// cache and runtime bases are resolved (`resolve_base`, shared with the
/// filesystem host's staging bases), so a linked home such as `/home` ->
/// `/var/home` keeps its disk-backed cache root, while the
/// `midnight-explorer/<name>` directories below a base are never resolved. Only
/// content that uses the cache looks it up: resolving a base on a network home
/// can block, and reports must not wait for a cache they skip. The cache root
/// still fails on an NFS home without `O_TMPFILE`, and sessions without
/// `XDG_RUNTIME_DIR` (su, sudo, ssh or containers) have no runtime root, so
/// those reach the temporary directory.
fn from_environment(
    variable: impl Fn(&str) -> Option<OsString>,
    canonical: impl Fn(&Path) -> Option<PathBuf>,
    uid: u32,
    temporary: PathBuf,
    content: SpoolContent,
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
        cache: cache
            .filter(|_| content.uses_cache())
            .map(|base| resolve_base(&base, &canonical)),
        runtime: absolute("XDG_RUNTIME_DIR").map(|base| resolve_base(&base, &canonical)),
        // The private-root check compares the owner with the effective user ID,
        // which `USER` need not name (`su` keeps it), so that ID is the label.
        account: Some(uid.to_string()),
        temporary,
    }
}
