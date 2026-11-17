//! Windows: `%TEMP%` is already a per-user directory with a user-only ACL.
use std::path::PathBuf;

pub(super) fn root(name: &str) -> PathBuf {
    std::env::temp_dir().join(name)
}

#[cfg(test)]
mod tests {
    #[test]
    fn spool_root_stays_inside_the_per_user_temporary_directory() {
        let root = super::root("spool");
        assert!(root.is_absolute());
        assert_eq!(root.parent(), Some(std::env::temp_dir().as_path()));
    }
}
