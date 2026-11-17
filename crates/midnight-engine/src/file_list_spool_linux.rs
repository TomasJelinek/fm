//! Per-user File List spool roots. A fixed name in the shared `/tmp` could be
//! pre-created by another account; the private-root check then correctly refuses
//! it, which would deny every file-list save. The preferred candidates belong to
//! this user; the shared temporary directory is only the last resort.
use std::{ffi::OsString, path::PathBuf};

pub(super) fn candidates() -> Vec<PathBuf> {
    candidates_from(
        |name| std::env::var_os(name),
        effective_uid(),
        std::env::temp_dir(),
    )
}

/// The XDG cache is disk-backed. The runtime directory (tmpfs) is the fallback for a
/// cache filesystem without anonymous files, such as an NFS home. Sessions without
/// either (su, sudo, ssh or containers) and a cache refused by the no-link walk
/// (a symlinked home component) still need a root: the temporary directory under
/// this account's numeric ID. Another account can occupy that predictable name, but
/// the private-root check then refuses it, so that only matters when both
/// per-user roots are unusable.
fn candidates_from(
    variable: impl Fn(&str) -> Option<OsString>,
    uid: Option<u32>,
    temporary: PathBuf,
) -> Vec<PathBuf> {
    let absolute = |name: &str| {
        variable(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    let cache =
        absolute("XDG_CACHE_HOME").or_else(|| absolute("HOME").map(|home| home.join(".cache")));
    let runtime = absolute("XDG_RUNTIME_DIR");
    let shared = uid
        .filter(|_| temporary.is_absolute())
        .map(|uid| temporary.join(format!("midnight-explorer-{uid}-file-list")));
    cache
        .map(|cache| cache.join("midnight-explorer").join("file-list"))
        .into_iter()
        .chain(runtime.map(|runtime| runtime.join("midnight-explorer-file-list")))
        .chain(shared)
        .collect()
}

/// The effective user ID, which the private-root check compares with the owner.
fn effective_uid() -> Option<u32> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let ids = status.lines().find_map(|line| line.strip_prefix("Uid:"))?;
    ids.split_whitespace().nth(1)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots(variables: &[(&str, &str)], uid: Option<u32>) -> Vec<PathBuf> {
        candidates_from(
            |name| {
                variables
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| OsString::from(value))
            },
            uid,
            PathBuf::from("/tmp"),
        )
    }

    #[test]
    fn file_list_spool_roots_are_per_user_and_reach_the_shared_temporary_directory_last() {
        assert_eq!(
            roots(
                &[
                    ("XDG_CACHE_HOME", "/home/u/cache"),
                    ("HOME", "/home/u"),
                    ("XDG_RUNTIME_DIR", "/run/user/1000"),
                    ("TMPDIR", "/tmp"),
                ],
                Some(1000)
            ),
            [
                PathBuf::from("/home/u/cache/midnight-explorer/file-list"),
                PathBuf::from("/run/user/1000/midnight-explorer-file-list"),
                PathBuf::from("/tmp/midnight-explorer-1000-file-list"),
            ]
        );
        assert_eq!(
            roots(&[("XDG_CACHE_HOME", "relative"), ("HOME", "/home/u")], None),
            [PathBuf::from("/home/u/.cache/midnight-explorer/file-list")]
        );
        // su/sudo/ssh/container sessions without XDG directories keep a root.
        assert_eq!(
            roots(&[("HOME", "relative"), ("TMPDIR", "/tmp")], Some(0)),
            [PathBuf::from("/tmp/midnight-explorer-0-file-list")]
        );
        assert!(roots(&[("HOME", "relative")], None).is_empty());
    }

    #[test]
    fn effective_uid_is_the_owner_of_files_this_process_creates() {
        use std::os::unix::fs::MetadataExt;
        let file = tempfile::tempfile().unwrap();
        assert_eq!(effective_uid(), Some(file.metadata().unwrap().uid()));
    }
}
