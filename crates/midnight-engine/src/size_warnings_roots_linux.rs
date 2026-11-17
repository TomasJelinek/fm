//! Linux spool bases: the XDG base directories, then the temporary directory
//! (usually the shared `/tmp`) under this account's effective user ID.
use super::Session;
use std::{ffi::OsString, path::PathBuf};

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

#[cfg(test)]
mod tests {
    use super::super::{SpoolContent, SpoolRoot, create_private_spool, spool_roots};
    use super::*;
    use midnight_worker_platform::spool::PrivateSpoolDirectory;
    use std::{
        fs::Permissions,
        os::unix::fs::{MetadataExt, PermissionsExt},
        path::Path,
    };

    fn session(variables: &[(&str, &str)], uid: Option<u32>) -> Session {
        from_environment(
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

    fn kept(path: impl Into<PathBuf>) -> SpoolRoot {
        SpoolRoot {
            path: path.into(),
            per_process: false,
        }
    }

    fn per_process(path: impl Into<PathBuf>) -> SpoolRoot {
        SpoolRoot {
            path: path.into(),
            per_process: true,
        }
    }

    #[test]
    fn spool_roots_use_xdg_directories_first_and_the_temporary_directory_last() {
        let desktop = session(
            &[
                ("XDG_CACHE_HOME", "/home/u/cache"),
                ("HOME", "/home/u"),
                ("XDG_RUNTIME_DIR", "/run/user/1000"),
            ],
            Some(1000),
        );
        assert_eq!(
            spool_roots(&desktop, 42, "file-list", SpoolContent::Large),
            [
                kept("/home/u/cache/midnight-explorer/file-list"),
                kept("/run/user/1000/midnight-explorer/file-list"),
                kept("/tmp/midnight-explorer-1000-file-list"),
                per_process("/tmp/midnight-explorer-file-list-42"),
            ]
        );
        // Small reports skip the disk-backed cache.
        assert_eq!(
            spool_roots(&desktop, 42, "reports", SpoolContent::Reports),
            [
                kept("/run/user/1000/midnight-explorer/reports"),
                kept("/tmp/midnight-explorer-1000-reports"),
                per_process("/tmp/midnight-explorer-reports-42"),
            ]
        );
        // Relative XDG variables are ignored; the cache defaults to ~/.cache.
        assert_eq!(
            spool_roots(
                &session(
                    &[
                        ("XDG_CACHE_HOME", "relative"),
                        ("HOME", "/home/u"),
                        ("XDG_RUNTIME_DIR", "run"),
                    ],
                    Some(1000),
                ),
                7,
                "file-list",
                SpoolContent::Large,
            ),
            [
                kept("/home/u/.cache/midnight-explorer/file-list"),
                kept("/tmp/midnight-explorer-1000-file-list"),
                per_process("/tmp/midnight-explorer-file-list-7"),
            ]
        );
        // su/sudo/ssh/container sessions without a home, XDG directories or
        // procfs still keep the per-process temporary root.
        assert_eq!(
            spool_roots(
                &session(&[("HOME", "relative")], None),
                7,
                "file-list",
                SpoolContent::Large
            ),
            [per_process("/tmp/midnight-explorer-file-list-7")]
        );
    }

    #[test]
    fn effective_uid_is_the_owner_of_files_this_process_creates() {
        let file = tempfile::tempfile().unwrap();
        assert_eq!(effective_uid(), Some(file.metadata().unwrap().uid()));
    }

    /// A directory that another account owns: as root, a new owner-only
    /// directory given to `nobody`; otherwise `/`, which belongs to root.
    fn foreign_directory(parent: &Path) -> PathBuf {
        if effective_uid() != Some(0) {
            return PathBuf::from("/");
        }
        let foreign = parent.join("foreign");
        std::fs::create_dir(&foreign).unwrap();
        std::fs::set_permissions(&foreign, Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::chown(&foreign, Some(65_534), Some(65_534)).unwrap();
        foreign
    }

    #[test]
    fn linked_foreign_and_shared_roots_are_refused_for_the_last_candidate() {
        let temporary = tempfile::tempdir().unwrap();
        // This user's own private directory, reached through a link that another
        // account could have planted under the expected name.
        let target = temporary.path().join("target");
        drop(PrivateSpoolDirectory::open(&target).unwrap());
        let linked = temporary.path().join("linked");
        std::os::unix::fs::symlink(&target, &linked).unwrap();
        let foreign = foreign_directory(temporary.path());
        // An own directory that other accounts may enter.
        let shared = temporary.path().join("shared");
        std::fs::create_dir(&shared).unwrap();
        std::fs::set_permissions(&shared, Permissions::from_mode(0o755)).unwrap();
        let last = temporary.path().join("midnight-explorer-reports-7");
        let roots = [
            kept(&linked),
            kept(&foreign),
            kept(&shared),
            per_process(&last),
        ];
        let mut spool = create_private_spool(&roots, PrivateSpoolDirectory::create).unwrap();
        // Creation stops at the first usable root and opening creates a missing
        // one, so every earlier candidate was refused.
        let metadata = std::fs::symlink_metadata(&last).unwrap();
        assert!(metadata.is_dir());
        assert_eq!(metadata.mode() & 0o777, 0o700);
        assert_eq!(spool.append(0, "東京 list".as_bytes()).unwrap(), 11);
        spool.finish().unwrap();
        let mut bytes = [0; 11];
        assert_eq!(spool.read_at(0, &mut bytes).unwrap(), 11);
        assert_eq!(&bytes, "東京 list".as_bytes());
        // The refused directories were neither used nor changed.
        assert!(std::fs::symlink_metadata(&linked).unwrap().is_symlink());
        assert_eq!(std::fs::read_dir(&target).unwrap().count(), 0);
        assert_eq!(std::fs::metadata(&shared).unwrap().mode() & 0o777, 0o755);
        drop(spool);
        assert!(
            !last.exists(),
            "the last spool removes its per-process root"
        );
        let error = create_private_spool(&roots[..3], PrivateSpoolDirectory::create)
            .err()
            .expect("no private root among the refused candidates");
        assert!(error.to_string().contains("shared"), "{error}");
    }
}
