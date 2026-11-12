use super::*;
use crate::spool::MAX_IO;

fn spool() -> (tempfile::TempDir, AppendSpool) {
    let temporary = tempfile::tempdir().unwrap();
    let root = PrivateSpoolDirectory::open(&temporary.path().join("private")).unwrap();
    (temporary, root.create_append_only().unwrap())
}

fn read(source: &SpoolPrefix) -> Vec<u8> {
    let mut result = vec![0; source.length() as usize + 8];
    let count = source.read_at(0, &mut result).unwrap();
    result.truncate(count);
    result
}

#[test]
fn prefix_remains_frozen_after_append_and_writer_retirement() {
    let (_temporary, mut spool) = spool();
    spool.append(0, "資料 🦀\n".as_bytes()).unwrap();
    let end = spool.length();
    let before = spool.prefix(0, end).unwrap();
    spool.append(end, b"next record\n").unwrap();
    let after = spool.prefix(end, spool.length()).unwrap();
    assert_eq!(read(&before), "資料 🦀\n".as_bytes());
    assert_eq!(read(&after), b"next record\n");
    drop(spool);
    assert_eq!(read(&before), "資料 🦀\n".as_bytes());
    assert_eq!(read(&after), b"next record\n");
}

#[test]
fn concurrent_prefix_reads_cannot_redirect_the_append_position() {
    let (_temporary, mut spool) = spool();
    spool.append(0, b"initial record\n").unwrap();
    let prefix = spool.prefix(0, spool.length()).unwrap();
    let reader = std::thread::spawn(move || {
        for _ in 0..1_000 {
            assert_eq!(read(&prefix), b"initial record\n");
        }
    });
    let mut expected = b"initial record\n".to_vec();
    for i in 0..1_000 {
        let next = format!("record {i}\n");
        spool.append(spool.length(), next.as_bytes()).unwrap();
        expected.extend_from_slice(next.as_bytes());
    }
    reader.join().unwrap();
    assert_eq!(read(&spool.prefix(0, spool.length()).unwrap()), expected);
}

#[test]
fn invalid_offsets_and_chunks_do_not_expose_or_overwrite_file_bytes() {
    let (_temporary, mut spool) = spool();
    assert!(spool.append(0, &[]).is_err());
    assert!(spool.append(0, &vec![0; MAX_IO + 1]).is_err());
    spool.append(0, b"abc").unwrap();
    assert!(spool.append(0, b"overwrite").is_err());
    assert!(spool.prefix(0, 4).is_err());
    assert!(spool.prefix(3, 2).is_err());
    let prefix = spool.prefix(1, 3).unwrap();
    assert_eq!(read(&prefix), b"bc");
    assert_eq!(prefix.read_at(10, &mut [0; 1]).unwrap(), 0);
    assert!(prefix.read_at(0, &mut vec![0; MAX_IO + 1]).is_err());
    spool.append(3, b"d").unwrap();
    assert_eq!(read(&spool.prefix(0, 4).unwrap()), b"abcd");
}

#[test]
fn late_partial_disk_failure_keeps_only_previously_committed_prefix_available() {
    let (_temporary, mut spool) = spool();
    spool.append(0, b"complete record\n").unwrap();
    let committed = spool.length();
    let earlier = spool.prefix(0, committed).unwrap();
    let error = spool
        .append_with(committed, b"incomplete record\n", |file, bytes| {
            file.write_all(&bytes[..4])?;
            Err(io::ErrorKind::StorageFull.into())
        })
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::StorageFull);
    assert_eq!(spool.length(), committed);
    assert!(spool.append(committed, b"retry").is_err());
    assert!(spool.prefix(0, committed + 4).is_err());
    assert_eq!(
        read(&spool.prefix(0, committed).unwrap()),
        b"complete record\n"
    );
    drop(spool);
    assert_eq!(read(&earlier), b"complete record\n");
}
