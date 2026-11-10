use crate::spool::PrivateSpoolDirectory;
use std::{fs::OpenOptions, io};

#[test]
fn actual_enospc_from_the_os_poisoning_survives_later_file_replacement() {
    let temporary = tempfile::tempdir().unwrap();
    let root = PrivateSpoolDirectory::open(&temporary.path().join("private")).unwrap();
    let mut spool = root.create().unwrap();
    let original = std::mem::replace(
        &mut spool.file,
        OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/full")
            .unwrap(),
    );
    assert_eq!(
        spool.append(0, b"content").unwrap_err().kind(),
        io::ErrorKind::StorageFull
    );
    spool.file = original;
    assert!(spool.append(0, b"retry").is_err());
    assert!(spool.finish().is_err());
    assert!(spool.read_at(0, &mut [0; 1]).is_err());
}

#[test]
fn anonymous_inode_is_private_unlinked_and_not_inheritable() {
    use std::os::unix::fs::MetadataExt;
    let temporary = tempfile::tempdir().unwrap();
    let root = PrivateSpoolDirectory::open(&temporary.path().join("private")).unwrap();
    let spool = root.create().unwrap();
    let metadata = spool.file.metadata().unwrap();
    assert_eq!(metadata.nlink(), 0);
    assert_eq!(metadata.mode() & 0o777, 0o600);
    assert!(
        rustix::io::fcntl_getfd(&spool.file)
            .unwrap()
            .contains(rustix::io::FdFlags::CLOEXEC)
    );
}
