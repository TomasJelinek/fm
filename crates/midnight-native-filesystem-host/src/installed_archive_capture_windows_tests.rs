use std::path::Path;

/// The plugin's revision prefix, the in-process (legacy) prefix, the payload length and
/// the payload offset of the file length. Payload bytes 32..40 hold the LastWriteTime
/// and the last 8 bytes the ChangeTime.
pub(super) const LAYOUT: (&[u8], &[u8], usize, usize) =
    (b"native.windows.read.v1", b"windows-file-v1", 48, 24);

/// What endpoint protection did to the reported archive: a new alternate data stream,
/// with the main stream's length and LastWriteTime as before. Writing a stream moves the
/// LastWriteTime as well, so it is set back, as that protection does; NTFS moves the
/// ChangeTime on both.
pub(super) fn tag(path: &Path) {
    let modified = std::fs::metadata(path).unwrap().modified().unwrap();
    let mut stream = path.as_os_str().to_owned();
    stream.push(":midnight.test.tag");
    std::fs::write(&stream, b"tagged").unwrap();
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(modified)
        .unwrap();
}
