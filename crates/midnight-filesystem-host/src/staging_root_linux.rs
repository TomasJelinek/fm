//! Per-user spool roots. A fixed name in the shared `/tmp` could be pre-created by
//! another account; the private-root check then correctly refuses it, which would
//! deny every staged archive read. Both candidates are owned by this user alone.
use std::{ffi::OsString, path::PathBuf};

pub(super) fn candidates() -> Vec<PathBuf> {
    candidates_from(|name| std::env::var_os(name))
}

/// The XDG cache is disk-backed. The runtime directory (tmpfs) is the fallback for a
/// cache filesystem without anonymous files, such as an NFS home.
fn candidates_from(variable: impl Fn(&str) -> Option<OsString>) -> Vec<PathBuf> {
    let absolute = |name: &str| {
        variable(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    let cache =
        absolute("XDG_CACHE_HOME").or_else(|| absolute("HOME").map(|home| home.join(".cache")));
    let runtime = absolute("XDG_RUNTIME_DIR");
    cache
        .map(|cache| cache.join("midnight-explorer").join("archive-spool"))
        .into_iter()
        .chain(runtime.map(|runtime| runtime.join("midnight-explorer-archive-spool")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots(variables: &[(&str, &str)]) -> Vec<PathBuf> {
        candidates_from(|name| {
            variables
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        })
    }

    #[test]
    fn spool_roots_are_per_user_and_never_the_shared_temporary_directory() {
        assert_eq!(
            roots(&[
                ("XDG_CACHE_HOME", "/home/u/cache"),
                ("HOME", "/home/u"),
                ("XDG_RUNTIME_DIR", "/run/user/1000"),
            ]),
            [
                PathBuf::from("/home/u/cache/midnight-explorer/archive-spool"),
                PathBuf::from("/run/user/1000/midnight-explorer-archive-spool"),
            ]
        );
        assert_eq!(
            roots(&[("XDG_CACHE_HOME", "relative"), ("HOME", "/home/u")]),
            [PathBuf::from(
                "/home/u/.cache/midnight-explorer/archive-spool"
            )]
        );
        assert!(roots(&[("HOME", "relative"), ("XDG_RUNTIME_DIR", "run")]).is_empty());
    }
}
