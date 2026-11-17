//! Per-user spool roots. A fixed name in the shared `/tmp` could be pre-created by
//! another account; the private-root check then correctly refuses it, which would
//! deny every staged archive read. Every candidate is scoped to this user, so a
//! directory another account pre-creates only disables that one candidate.
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

pub(super) fn candidates() -> Vec<PathBuf> {
    candidates_from(
        |name| std::env::var_os(name),
        |path| std::fs::canonicalize(path).ok(),
        std::env::temp_dir(),
        effective_user(),
    )
}

/// In order: the disk-backed XDG cache (or `~/.cache`); a user-scoped name in the
/// temporary directory, which also serves sessions without a usable home, such as an
/// NFS home without anonymous files; `$XDG_RUNTIME_DIR`, usually a small tmpfs meant
/// for small runtime files, as the last resort for possibly large decoded members.
fn candidates_from(
    variable: impl Fn(&str) -> Option<OsString>,
    canonical: impl Fn(&Path) -> Option<PathBuf>,
    temporary: PathBuf,
    user: Option<u32>,
) -> Vec<PathBuf> {
    let absolute = |name: &str| {
        variable(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    let cache =
        absolute("XDG_CACHE_HOME").or_else(|| absolute("HOME").map(|home| home.join(".cache")));
    let temporary = user
        .filter(|_| temporary.is_absolute())
        .map(|user| (temporary, format!("midnight-explorer-{user}-archive-spool")));
    let runtime = absolute("XDG_RUNTIME_DIR");
    cache
        .map(|cache| (cache, "midnight-explorer/archive-spool".to_owned()))
        .into_iter()
        .chain(temporary)
        .chain(runtime.map(|runtime| (runtime, "midnight-explorer-archive-spool".to_owned())))
        .map(|(base, spool)| resolve(&base, &canonical).join(spool))
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
    use std::os::unix::fs::MetadataExt;

    fn roots(variables: &[(&str, &str)], user: Option<u32>) -> Vec<PathBuf> {
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
        )
    }

    #[test]
    fn spool_roots_are_per_user_and_never_a_shared_temporary_name() {
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
                PathBuf::from("/mnt/disk/cache/midnight-explorer/archive-spool"),
                PathBuf::from("/tmp/midnight-explorer-1000-archive-spool"),
                // A base that cannot be resolved is used as entered.
                PathBuf::from("/run/user/1000/midnight-explorer-archive-spool"),
            ]
        );
        // A linked home resolves before its missing `.cache` is appended.
        assert_eq!(
            roots(&[("XDG_CACHE_HOME", "relative"), ("HOME", "/home/u")], None),
            [PathBuf::from(
                "/var/home/u/.cache/midnight-explorer/archive-spool"
            )]
        );
        // Without any usable variable only the user-scoped temporary name remains.
        assert_eq!(
            roots(&[("HOME", "relative"), ("XDG_RUNTIME_DIR", "run")], Some(0)),
            [PathBuf::from("/tmp/midnight-explorer-0-archive-spool")]
        );
        assert!(roots(&[], None).is_empty());
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
        assert!(super::super::Staging::open(&link.join(spool)).is_err());
        let roots = candidates_from(
            |name| (name == "XDG_CACHE_HOME").then(|| link.clone().into_os_string()),
            |path| std::fs::canonicalize(path).ok(),
            PathBuf::from("relative"),
            None,
        );
        let real = real.canonicalize().unwrap();
        assert_eq!(roots, [real.join(spool)]);
        super::super::Staging::open_first(roots).unwrap();
        assert!(real.join(spool).is_dir());
    }
}
