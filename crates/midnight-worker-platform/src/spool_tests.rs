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

fn read_all(spool: &Spool, offset: u64, length: usize) -> io::Result<Vec<u8>> {
    let mut output = vec![0; length];
    let mut filled = 0;
    while filled < length {
        let end = (filled + MAX_IO).min(length);
        let count = spool.read_at(offset + filled as u64, &mut output[filled..end])?;
        if count == 0 {
            break;
        }
        filled += count;
    }
    output.truncate(filled);
    Ok(output)
}

#[test]
fn sealed_reads_return_appended_bytes_across_chunks_and_coalesce_small_appends() {
    let temporary = tempfile::tempdir().unwrap();
    let root = PrivateSpoolDirectory::open(&temporary.path().join("private")).unwrap();
    let mut spool = root.create().unwrap();
    let content: Vec<u8> = (0..700_000u32).map(|i| (i * 31 % 251) as u8).collect();
    // Small appends first, then maximal ones, as decoders and copies produce them.
    let mut offset = 0;
    for piece in content[..300_000]
        .chunks(1000)
        .chain(content[300_000..].chunks(MAX_IO))
    {
        offset = spool.append(offset, piece).unwrap();
    }
    spool.finish().unwrap();
    // One checksum per block, whatever the appends: 4 bytes per 64 KiB.
    assert_eq!(
        spool.blocks.len() as u64,
        (content.len() as u64).div_ceil(BLOCK),
        "300 small appends share blocks"
    );
    assert_eq!(read_all(&spool, 0, content.len()).unwrap(), content);
    let block = BLOCK as usize;
    for (start, length) in [
        (0, 1),
        // Across one block boundary, and across several.
        (block - 1, 2),
        (block - 3, block + 6),
        (262_143, 3),
        (299_999, 200_002),
        // The last, partial block, and past the end.
        (content.len() - content.len() % block - 1, 3),
        (699_000, 5000),
    ] {
        let expected = &content[start..(start + length).min(content.len())];
        assert_eq!(read_all(&spool, start as u64, length).unwrap(), expected);
    }
    assert_eq!(spool.read_at(content.len() as u64, &mut [0; 8]).unwrap(), 0);
}

#[test]
fn a_changed_block_fails_only_the_reads_that_touch_it() {
    use std::io::{Seek, SeekFrom};
    let temporary = tempfile::tempdir().unwrap();
    let root = PrivateSpoolDirectory::open(&temporary.path().join("private")).unwrap();
    let mut spool = root.create().unwrap();
    // One append spanning several blocks.
    spool.append(0, &[5u8; MAX_IO]).unwrap();
    spool.finish().unwrap();
    let block = BLOCK as usize;
    let mut file = spool.file.try_clone().unwrap();
    file.seek(SeekFrom::Start(BLOCK + 10)).unwrap();
    file.write_all(&[6]).unwrap();
    // The blocks before and after it, inside the same append, are still checked as fine.
    assert_eq!(read_all(&spool, 0, block).unwrap(), vec![5; block]);
    assert_eq!(read_all(&spool, 2 * BLOCK, 16).unwrap(), vec![5; 16]);
    for (start, length) in [(BLOCK + 10, 1), (BLOCK - 1, 2), (0, MAX_IO)] {
        let error = read_all(&spool, start, length).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{start} {length}");
    }
}

#[test]
fn bytes_changed_after_they_were_appended_are_never_returned() {
    use std::io::{Seek, SeekFrom};
    let temporary = tempfile::tempdir().unwrap();
    let root = PrivateSpoolDirectory::open(&temporary.path().join("private")).unwrap();
    let mut spool = root.create().unwrap();
    let content = vec![7u8; 3 * MAX_IO];
    let mut offset = 0;
    for piece in content.chunks(MAX_IO) {
        offset = spool.append(offset, piece).unwrap();
    }
    spool.finish().unwrap();
    // Stand-in for write-back that failed or returned other data from storage.
    let mut file = spool.file.try_clone().unwrap();
    file.seek(SeekFrom::Start(MAX_IO as u64 + 10)).unwrap();
    file.write_all(&[8]).unwrap();
    let error = read_all(&spool, MAX_IO as u64 - 4, 8).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    // Chunks that did not change stay readable.
    assert_eq!(
        read_all(&spool, 2 * MAX_IO as u64 + 20, 30).unwrap(),
        vec![7; 30]
    );
    assert_eq!(read_all(&spool, 0, 16).unwrap(), vec![7; 16]);
}

#[test]
fn clones_share_the_directory_and_count_every_created_spool() {
    let temporary = tempfile::tempdir().unwrap();
    let root = PrivateSpoolDirectory::open(&temporary.path().join("private")).unwrap();
    let clone = root.clone();
    assert_eq!(root.created_spools(), 0);
    drop(root.create().unwrap());
    drop(clone.create_append_only().unwrap());
    assert_eq!((root.created_spools(), clone.created_spools()), (2, 2));
    assert!(root.is_same(&clone));
    // Opening the same path again is a separate capability with its own count.
    let other = PrivateSpoolDirectory::open(&temporary.path().join("private")).unwrap();
    assert!(!root.is_same(&other));
    assert_eq!(other.created_spools(), 0);
}
