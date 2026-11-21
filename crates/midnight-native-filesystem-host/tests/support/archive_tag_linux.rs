use std::path::Path;

/// A mode change, which moves only the ctime.
pub fn tag(path: &Path) {
    let writable = std::fs::metadata(path).unwrap().permissions();
    let mut read_only = writable.clone();
    read_only.set_readonly(true);
    std::fs::set_permissions(path, read_only).unwrap();
    std::fs::set_permissions(path, writable).unwrap();
}
