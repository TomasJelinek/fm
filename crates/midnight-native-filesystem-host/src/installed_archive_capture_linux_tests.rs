use std::path::Path;

/// The plugin's revision prefix, the in-process (legacy) prefix, the payload length and
/// the payload offset of the file length. Payload bytes 24..40 hold the mtime (seconds,
/// nanoseconds) and the last 16 bytes the ctime.
pub(super) const LAYOUT: (&[u8], &[u8], usize, usize) =
    (b"native.linux.read.v1", b"linux-file-v1", 56, 16);

/// A mode change, which moves only the ctime.
pub(super) fn tag(path: &Path) {
    let writable = std::fs::metadata(path).unwrap().permissions();
    let mut read_only = writable.clone();
    read_only.set_readonly(true);
    std::fs::set_permissions(path, read_only).unwrap();
    std::fs::set_permissions(path, writable).unwrap();
}
