//! Linux: the default temporary directory `/tmp` is shared by all accounts.
use std::{os::unix::fs::MetadataExt, path::PathBuf};

pub(super) fn root(name: &str) -> PathBuf {
    select(
        std::env::var_os("TMPDIR").is_some(),
        std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
        std::env::temp_dir(),
        // `/proc/self` belongs to the effective user of this process.
        std::fs::metadata("/proc/self")
            .ok()
            .map(|metadata| metadata.uid()),
        name,
    )
}

/// An explicitly chosen TMPDIR wins (test runs keep their output there). Without
/// one, the session's XDG runtime directory is owner-only by specification. A
/// shared temporary directory gets an owner-specific name, so another account's
/// directory cannot collide with ours.
fn select(
    explicit_tmpdir: bool,
    runtime: Option<PathBuf>,
    temp: PathBuf,
    uid: Option<u32>,
    name: &str,
) -> PathBuf {
    if !explicit_tmpdir && let Some(runtime) = runtime.filter(|path| path.is_absolute()) {
        return runtime.join(name);
    }
    match uid {
        Some(uid) => temp.join(format!("{name}-{uid}")),
        None => temp.join(name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_temporary_roots_are_named_per_user_and_runtime_roots_are_private() {
        let temp = PathBuf::from("/tmp");
        let runtime = Some(PathBuf::from("/run/user/1000"));
        assert_eq!(
            select(false, runtime.clone(), temp.clone(), Some(1000), "spool"),
            PathBuf::from("/run/user/1000/spool")
        );
        assert_eq!(
            select(
                true,
                runtime,
                PathBuf::from("/build/tmp"),
                Some(1000),
                "spool"
            ),
            PathBuf::from("/build/tmp/spool-1000"),
            "an explicit TMPDIR is honored, still with a per-user name"
        );
        assert_eq!(
            select(false, None, temp.clone(), Some(1001), "spool"),
            PathBuf::from("/tmp/spool-1001")
        );
        assert_ne!(
            select(false, None, temp.clone(), Some(1000), "spool"),
            select(false, None, temp.clone(), Some(1001), "spool"),
            "two accounts never share one spool root"
        );
        assert_eq!(
            select(
                false,
                Some(PathBuf::from("relative")),
                temp,
                Some(7),
                "spool"
            ),
            PathBuf::from("/tmp/spool-7"),
            "a relative runtime directory is ignored"
        );
        assert!(root("spool").is_absolute());
    }
}
