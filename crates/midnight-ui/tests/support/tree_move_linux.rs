//! Private mode for this owned Linux fixture only.
use std::{fs, os::unix::fs::PermissionsExt, path::Path};

pub fn secure(path: &Path) {
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}
