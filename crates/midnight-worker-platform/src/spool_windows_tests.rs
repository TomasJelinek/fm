use crate::spool::PrivateSpoolDirectory;
use std::{fs::File, io, os::windows::io::AsRawHandle};
use windows_sys::Win32::Foundation::{GetHandleInformation, HANDLE_FLAG_INHERIT};

#[test]
fn private_spool_handle_is_not_inheritable() {
    let temporary = tempfile::tempdir().unwrap();
    let root = PrivateSpoolDirectory::open(&temporary.path().join("private")).unwrap();
    let spool = root.create().unwrap();
    let mut flags = 0;
    // SAFETY: live owned spool file handle and valid flags output.
    assert_ne!(
        unsafe { GetHandleInformation(spool.file.as_raw_handle(), &mut flags) },
        0
    );
    assert_eq!(flags & HANDLE_FLAG_INHERIT, 0);
}

#[test]
fn actual_write_and_flush_access_errors_permanently_poison_the_spool() {
    let temporary = tempfile::tempdir().unwrap();
    let root = PrivateSpoolDirectory::open(&temporary.path().join("private")).unwrap();
    let read_only = temporary.path().join("read-only");
    std::fs::write(&read_only, b"untouched").unwrap();
    for flush in [false, true] {
        let mut spool = root.create().unwrap();
        let original = std::mem::replace(&mut spool.file, File::open(&read_only).unwrap());
        let error = if flush {
            spool.finish().unwrap_err()
        } else {
            spool.append(0, b"data").unwrap_err()
        };
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        spool.file = original;
        assert!(spool.append(0, b"retry").is_err());
        assert!(spool.finish().is_err());
        assert!(spool.read_at(0, &mut [0; 1]).is_err());
    }
    assert_eq!(std::fs::read(read_only).unwrap(), b"untouched");
}

#[test]
fn concurrent_shared_spool_directory_first_use_preserves_every_writer() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("shared").join("reports");
    let ready = std::sync::Arc::new(std::sync::Barrier::new(12));
    let threads: Vec<_> = (0..12u8)
        .map(|index| {
            let path = path.clone();
            let ready = ready.clone();
            std::thread::spawn(move || -> io::Result<_> {
                ready.wait();
                let directory = PrivateSpoolDirectory::open(&path)?;
                let mut spool = directory.create()?;
                spool.append(0, &[index])?;
                spool.finish()?;
                let mut byte = [0];
                assert_eq!(spool.read_at(0, &mut byte)?, 1);
                assert_eq!(byte, [index]);
                Ok(spool)
            })
        })
        .collect();
    let results: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    let errors: Vec<_> = results
        .iter()
        .filter_map(|result| result.as_ref().err())
        .collect();
    assert!(
        errors.is_empty(),
        "concurrent first use lost writers: {errors:?}"
    );
    assert!(
        std::fs::rename(&path, temporary.path().join("moved")).is_err(),
        "successful writers still pin the shared directory"
    );
    drop(results);
    assert_eq!(std::fs::read_dir(path).unwrap().count(), 0);
}
