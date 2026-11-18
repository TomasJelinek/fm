//! Linux spool bases chosen by the user or the system, such as `$XDG_CACHE_HOME`,
//! `~/.cache` or `$XDG_RUNTIME_DIR`. Every backend that lists private spool roots
//! below such a base resolves its links here, so one link policy serves them all.
use std::path::{Path, PathBuf};

#[cfg(test)]
#[path = "spool_base_linux_tests.rs"]
mod tests;

/// Resolve the links in `base`, a directory that the user or the system chose (such
/// as `/home` linked to `/var/home`, or a linked `~/.cache`), including trailing
/// components that do not exist yet: `canonical` (normally `fs::canonicalize`)
/// resolves the longest prefix that it can, and the missing names follow unchanged.
/// A base without any resolvable prefix is returned as entered.
///
/// Callers append their spool root names to the result and never resolve those, so
/// the private-root walk (`PrivateSpoolDirectory::open`), which follows no link,
/// still refuses a link at or below the spool root, and the root itself must be
/// owned by this user with mode 0700.
pub fn resolve_base(base: &Path, canonical: impl Fn(&Path) -> Option<PathBuf>) -> PathBuf {
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
