use std::path::Path;

/// What endpoint protection did to a reported archive: a new alternate data stream,
/// with the main stream's length and LastWriteTime as before. Writing a stream moves the
/// LastWriteTime as well, so it is set back, as that protection does; NTFS moves the
/// ChangeTime on both.
pub fn tag(path: &Path) {
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
