//! Per-user spool roots. A fixed name in the shared `/tmp` could be pre-created by
//! another account; the private-root check then correctly refuses it, which would
//! deny every staged archive read. Every candidate is scoped to this user or this
//! process, so a directory another account pre-creates only disables that candidate.
use midnight_worker_platform::spool::SpoolRoot;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

pub(super) fn candidates() -> Vec<SpoolRoot> {
    candidates_from(
        |name| std::env::var_os(name),
        |path| std::fs::canonicalize(path).ok(),
        std::env::temp_dir(),
        effective_user(),
        std::process::id(),
    )
}

/// In order: the disk-backed XDG cache (or `~/.cache`); a user-scoped name in the
/// temporary directory, which also serves sessions without a usable home, such as an
/// NFS home without anonymous files; a name for this process alone in the temporary
/// directory, for sessions whose cache is unusable when another account took the
/// user-scoped name; and `$XDG_RUNTIME_DIR`, usually a small tmpfs meant for small
/// runtime files, as the last resort for possibly large decoded members.
fn candidates_from(
    variable: impl Fn(&str) -> Option<OsString>,
    canonical: impl Fn(&Path) -> Option<PathBuf>,
    temporary: PathBuf,
    user: Option<u32>,
    process: u32,
) -> Vec<SpoolRoot> {
    let absolute = |name: &str| {
        variable(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    let cache =
        absolute("XDG_CACHE_HOME").or_else(|| absolute("HOME").map(|home| home.join(".cache")));
    let temporary = Some(temporary).filter(|temporary| temporary.is_absolute());
    let root = |base: &Option<PathBuf>, name: String, per_process| {
        base.as_ref().map(|base| SpoolRoot {
            path: resolve(base, &canonical).join(name),
            per_process,
        })
    };
    [
        root(&cache, "midnight-explorer/archive-spool".into(), false),
        user.and_then(|user| {
            root(
                &temporary,
                format!("midnight-explorer-{user}-archive-spool"),
                false,
            )
        }),
        root(
            &temporary,
            format!("midnight-explorer-archive-spool-{process}"),
            true,
        ),
        root(
            &absolute("XDG_RUNTIME_DIR"),
            "midnight-explorer-archive-spool".into(),
            false,
        ),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Resolve links in a base directory chosen by the user or the system, such as `/home`
/// linked to `/var/home` or a linked `~/.cache`, including trailing components that do
/// not exist yet. The private-root walk then follows no link. The spool directories
/// below the base are never resolved: a link there is refused, and the spool root
/// itself must be owned by this user with mode 0700.
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

/// The effective user ID from procfs, which the host already requires on Linux to
/// locate its own executable. None only omits the temporary-directory candidate.
fn effective_user() -> Option<u32> {
    effective_user_from(&std::fs::read_to_string("/proc/self/status").ok()?)
}

/// `Uid:` lists the real, effective, saved and filesystem user IDs.
fn effective_user_from(status: &str) -> Option<u32> {
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
    use super::*;
    use crate::staging::{Staging, ample_volume};
    use std::os::unix::fs::MetadataExt;

    fn roots(variables: &[(&str, &str)], user: Option<u32>) -> Vec<SpoolRoot> {
        candidates_from(
            |name| {
                variables
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| OsString::from(value))
            },
            |path| {
                [
                    ("/home/u", "/var/home/u"),
                    ("/data/cache", "/mnt/disk/cache"),
                    ("/tmp", "/tmp"),
                ]
                .iter()
                .find(|(link, _)| Path::new(link) == path)
                .map(|(_, target)| PathBuf::from(target))
            },
            PathBuf::from("/tmp"),
            user,
            42,
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
    fn spool_roots_are_never_a_shared_name_and_the_runtime_directory_comes_last() {
        let process = || per_process("/tmp/midnight-explorer-archive-spool-42");
        assert_eq!(
            roots(
                &[
                    ("XDG_CACHE_HOME", "/data/cache"),
                    ("HOME", "/home/u"),
                    ("XDG_RUNTIME_DIR", "/run/user/1000"),
                ],
                Some(1000)
            ),
            [
                kept("/mnt/disk/cache/midnight-explorer/archive-spool"),
                kept("/tmp/midnight-explorer-1000-archive-spool"),
                process(),
                // A base that cannot be resolved is used as entered.
                kept("/run/user/1000/midnight-explorer-archive-spool"),
            ]
        );
        // A linked home resolves before its missing `.cache` is appended.
        assert_eq!(
            roots(&[("XDG_CACHE_HOME", "relative"), ("HOME", "/home/u")], None),
            [
                kept("/var/home/u/.cache/midnight-explorer/archive-spool"),
                process(),
            ]
        );
        // Without any usable variable only the temporary directory's names remain.
        assert_eq!(
            roots(&[("HOME", "relative"), ("XDG_RUNTIME_DIR", "run")], Some(0)),
            [kept("/tmp/midnight-explorer-0-archive-spool"), process()]
        );
        assert_eq!(roots(&[], None), [process()]);
    }

    #[test]
    fn a_foreign_owned_root_falls_through_to_the_per_process_root() {
        let temporary = tempfile::tempdir().unwrap();
        let own = temporary.path().join("midnight-explorer-archive-spool-7");
        // `/` stands in for a user-scoped name that another account created first:
        // root owns it (and refuses its mode 0755 when the test itself runs as root).
        let staging = Staging::open_first([kept("/"), per_process(&own)], ample_volume).unwrap();
        assert!(own.is_dir());
        drop(staging);
        assert!(!own.exists(), "the last spool removes the per-process root");
    }

    #[test]
    fn effective_user_is_read_from_the_status_uid_line() {
        let status = "Name:\thost\nUmask:\t0022\nUid:\t1000\t1001\t1000\t1000\nGid:\t5\t6\t5\t5\n";
        assert_eq!(effective_user_from(status), Some(1001));
        assert_eq!(effective_user_from("Name:\thost\n"), None);
        assert_eq!(effective_user_from("Uid:\t1000\n"), None);
        // New files belong to the effective user, which the private-root check requires.
        let file = tempfile::NamedTempFile::new().unwrap();
        assert_eq!(
            effective_user(),
            Some(file.as_file().metadata().unwrap().uid())
        );
    }

    #[test]
    fn a_linked_cache_home_yields_a_usable_private_spool_root() {
        let temporary = tempfile::tempdir().unwrap();
        let real = temporary.path().join("real-cache");
        std::fs::create_dir(&real).unwrap();
        let link = temporary.path().join("linked-cache");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let spool = Path::new("midnight-explorer/archive-spool");
        // The private-root walk follows no link, so the entered spelling is unusable.
        assert!(Staging::open(&link.join(spool)).is_err());
        let roots = candidates_from(
            |name| (name == "XDG_CACHE_HOME").then(|| link.clone().into_os_string()),
            |path| std::fs::canonicalize(path).ok(),
            PathBuf::from("relative"),
            None,
            7,
        );
        let real = real.canonicalize().unwrap();
        assert_eq!(roots, [kept(real.join(spool))]);
        Staging::open_first(roots, ample_volume).unwrap();
        assert!(real.join(spool).is_dir());
    }
}
