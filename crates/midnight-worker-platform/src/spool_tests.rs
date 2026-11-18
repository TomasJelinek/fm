use super::*;

#[test]
fn partial_write_then_disk_full_permanently_blocks_sealing_and_reading() {
    let temporary = tempfile::tempdir().unwrap();
    let root = PrivateSpoolDirectory::open(&temporary.path().join("private")).unwrap();
    let mut spool = root.create().unwrap();
    spool.append(0, b"before").unwrap();
    let error = spool
        .append_with(6, b"partial-data", |file, bytes| {
            file.write_all(&bytes[..3])?;
            Err(io::ErrorKind::StorageFull.into())
        })
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::StorageFull);
    assert_eq!(spool.file.metadata().unwrap().len(), 9);
    assert_eq!(spool.length(), 6);
    assert!(spool.append(6, b"retry").is_err());
    assert!(spool.finish().is_err());
    assert!(spool.read_at(0, &mut [0; 9]).is_err());
}

#[test]
fn failed_flush_cannot_be_retried_into_a_readable_spool() {
    let temporary = tempfile::tempdir().unwrap();
    let root = PrivateSpoolDirectory::open(&temporary.path().join("private")).unwrap();
    let mut spool = root.create().unwrap();
    spool.append(0, b"data").unwrap();
    assert!(
        spool
            .finish_with(|_| Err(io::ErrorKind::StorageFull.into()))
            .is_err()
    );
    assert!(spool.finish().is_err());
    assert!(spool.read_at(0, &mut [0; 4]).is_err());
}

#[test]
fn the_free_space_reserve_is_the_larger_of_one_gibibyte_and_two_percent() {
    const GIB: u64 = 1 << 30;
    assert_eq!(free_space_reserve(0), GIB);
    assert_eq!(free_space_reserve(50 * GIB), GIB);
    assert_eq!(free_space_reserve(100 * GIB), 2 * GIB);
    assert_eq!(free_space_reserve(u64::MAX), u64::MAX / 50);
}

#[test]
fn signed_platform_offset_range_accepts_large_files_without_a_policy_quota() {
    assert_eq!(
        checked_end(16 * 1024 * 1024 * 1024, MAX_IO).unwrap(),
        16 * 1024 * 1024 * 1024 + MAX_IO as u64
    );
    assert_eq!(checked_end(i64::MAX as u64, 0).unwrap(), i64::MAX as u64);
    assert!(checked_end(i64::MAX as u64, 1).is_err());
}
