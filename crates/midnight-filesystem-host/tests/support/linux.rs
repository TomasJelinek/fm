use std::{fs, os::unix::fs::PermissionsExt, path::Path};

pub(super) fn secure_test_executable(path: &Path) {
    // DrvFS build files may carry 0777. Only the copied private test installation
    // is hardened; source/build paths and the production trust policy are unchanged.
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}
