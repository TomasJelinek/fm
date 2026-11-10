use super::*;
use crate::CapturedFile;
use std::{
    fs,
    io::{Read, Write},
    os::unix::ffi::OsStringExt,
};

fn fixture() -> tempfile::TempDir {
    let directory = tempfile::Builder::new()
        .prefix("viewer-file-")
        .tempdir()
        .unwrap();
    assert!(
        directory
            .path()
            .components()
            .any(|part| part.as_os_str() == "_build")
    );
    directory
}
fn fifo(path: &Path) {
    let name = CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: a fresh FIFO in this test's exclusively owned temporary directory.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
}
fn watch(path: &Path) -> File {
    // SAFETY: no borrowed pointers; successful descriptor transfers once to File.
    let file = owned(unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) }).unwrap();
    let name = CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: owned live descriptor and terminated owned fixture pathname.
    assert!(
        unsafe { libc::inotify_add_watch(file.as_raw_fd(), name.as_ptr(), libc::IN_OPEN) } >= 0
    );
    file
}
fn opened(watcher: &mut File) -> bool {
    let mut events = [0u8; 256];
    let count = match watcher.read(&mut events) {
        Ok(count) => count,
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => 0,
        Err(error) => panic!("{error}"),
    };
    count >= 16 && u32::from_ne_bytes(events[4..8].try_into().unwrap()) & libc::IN_OPEN != 0
}

#[test]
fn captured_revalidation_never_data_opens_a_replacement_fifo() {
    let fixture = fixture();
    let path = fixture.path().join("source");
    fs::write(&path, b"original").unwrap();
    let captured = CapturedFile::capture(&path).unwrap();
    fs::rename(&path, fixture.path().join("held")).unwrap();
    fifo(&path);
    let mut watcher = watch(&path);
    assert!(captured.revalidate().is_err());
    assert!(super::open_read(&path).is_err());
    assert!(
        !opened(&mut watcher),
        "FIFO rejection must precede any data open"
    );
}

#[test]
fn data_reopen_uses_the_qualified_inode_when_its_name_becomes_a_fifo() {
    let fixture = fixture();
    let path = fixture.path().join("source");
    fs::write(&path, b"original").unwrap();
    let probe = probe(&path).unwrap();
    fs::rename(&path, fixture.path().join("held")).unwrap();
    fifo(&path);
    let mut watcher = watch(&path);
    let mut file = reopen(&probe).unwrap();
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"original");
    assert!(
        !opened(&mut watcher),
        "data reopen must not resolve the replaced name"
    );
}

#[test]
fn native_capture_and_revalidation_preserve_a_raw_path_beyond_path_max() {
    let fixture = fixture();
    let mut path = fixture.path().to_owned();
    let mut parent = File::open(&path).unwrap();
    let component = CString::new("x".repeat(200)).unwrap();
    while path.as_os_str().as_bytes().len() < 4300 {
        // SAFETY: held owned parent, fixed terminated basename, exclusively owned fixture.
        assert_eq!(
            unsafe { libc::mkdirat(parent.as_raw_fd(), component.as_ptr(), 0o700) },
            0
        );
        // SAFETY: held parent and just-created directory; ownership transfers once.
        parent = owned(unsafe {
            libc::openat(
                parent.as_raw_fd(),
                component.as_ptr(),
                libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        })
        .unwrap();
        path.push(std::ffi::OsStr::from_bytes(component.as_bytes()));
    }
    let name = CString::new(vec![b'f', 0xff]).unwrap();
    // SAFETY: exclusive creation in the retained owned fixture directory.
    let mut file = owned(unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC,
            0o600,
        )
    })
    .unwrap();
    file.write_all(b"long raw source").unwrap();
    drop(file);
    path.push(std::ffi::OsString::from_vec(name.as_bytes().to_vec()));
    let captured = CapturedFile::capture(&path).unwrap();
    assert_eq!(captured.path(), path);
    assert_eq!(captured.read_at(0, 100).unwrap(), b"long raw source");
    captured.revalidate().unwrap();
}
