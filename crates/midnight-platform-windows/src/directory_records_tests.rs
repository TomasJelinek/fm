use super::*;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs::OpenOptions,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::OpenOptionsExt,
    },
    path::PathBuf,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_LIST_DIRECTORY, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE,
};

fn fixture() -> tempfile::TempDir {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../_build/tmp");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix("records-")
        .tempdir_in(root)
        .unwrap()
}

fn open_directory(path: &std::path::Path) -> File {
    OpenOptions::new()
        .access_mode(FILE_LIST_DIRECTORY)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .unwrap()
}

struct Seen {
    directory: bool,
    size: u64,
    modified: Option<SystemTime>,
}

fn read_all(directory: &File) -> (BTreeMap<OsString, Seen>, usize) {
    let mut cursor = Cursor::default();
    let mut seen = BTreeMap::new();
    let mut batches = 0;
    while !cursor.ended() {
        cursor
            .next_batch(directory, &mut |record| {
                let previous = seen.insert(
                    OsString::from_wide(record.name),
                    Seen {
                        directory: record.native.directory(),
                        size: record.size,
                        modified: record.modified(),
                    },
                );
                assert!(previous.is_none(), "each name is reported once");
                Ok(())
            })
            .unwrap();
        batches += 1;
    }
    (seen, batches)
}

#[test]
fn records_span_batches_and_match_native_metadata_without_opening_children() {
    let temp = fixture();
    // Long Unicode names (including an unpaired surrogate) overflow one 64 KiB batch.
    let mut expected = Vec::new();
    for index in 0..400u32 {
        let mut units: Vec<u16> = format!("entry-{index:04}-文档-").encode_utf16().collect();
        units.extend(std::iter::repeat_n(u16::from(b'x'), 100));
        if index % 7 == 0 {
            units.push(0xd800);
        }
        let name = OsString::from_wide(&units);
        let path = temp.path().join(&name);
        if index % 5 == 0 {
            std::fs::create_dir(&path).unwrap();
        } else {
            std::fs::write(&path, vec![b'a'; index as usize]).unwrap();
        }
        expected.push((name, path, index % 5 == 0));
    }
    let directory = open_directory(temp.path());
    let (seen, batches) = read_all(&directory);
    assert!(
        batches > 2,
        "the fixture must exercise several native batches"
    );
    assert_eq!(seen.len(), expected.len());
    for (name, path, is_directory) in expected {
        let record = &seen[&name];
        let metadata = std::fs::symlink_metadata(&path).unwrap();
        assert_eq!(record.directory, is_directory, "{path:?}");
        if !is_directory {
            assert_eq!(record.size, metadata.len(), "{path:?}");
        }
        assert_eq!(record.modified, metadata.modified().ok(), "{path:?}");
    }
}

#[test]
fn an_empty_directory_ends_without_records_and_stays_ended() {
    let temp = fixture();
    let directory = open_directory(temp.path());
    let (seen, _) = read_all(&directory);
    assert!(seen.is_empty());
    let mut cursor = Cursor::default();
    let mut visits = 0;
    while !cursor.ended() {
        cursor
            .next_batch(&directory, &mut |_| {
                visits += 1;
                Ok(())
            })
            .unwrap();
    }
    cursor
        .next_batch(&directory, &mut |_| panic!("an ended cursor reads nothing"))
        .unwrap();
    assert_eq!(visits, 0);
}

#[test]
fn a_visitor_error_stops_the_batch() {
    let temp = fixture();
    for name in ["a", "b", "c"] {
        std::fs::write(temp.path().join(name), b"").unwrap();
    }
    let directory = open_directory(temp.path());
    let mut visits = 0;
    let error = Cursor::default()
        .next_batch(&directory, &mut |_| {
            visits += 1;
            Err(io::Error::new(io::ErrorKind::Interrupted, "stop"))
        })
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(visits, 1);
}

fn record(buffer: &mut [u64], offset: usize, next: u32, name: &str) {
    let base = buffer.as_mut_ptr().cast::<u8>();
    let units: Vec<u16> = std::ffi::OsStr::new(name).encode_wide().collect();
    // SAFETY: test records are written inside the zeroed, aligned buffer.
    unsafe {
        let header = base.add(offset).cast::<FILE_ID_BOTH_DIR_INFO>();
        (*header).NextEntryOffset = next;
        (*header).FileNameLength = (units.len() * 2) as u32;
        std::ptr::copy_nonoverlapping(
            units.as_ptr(),
            base.add(offset + offset_of!(FILE_ID_BOTH_DIR_INFO, FileName))
                .cast::<u16>(),
            units.len(),
        );
    }
}

#[test]
fn malformed_native_records_are_rejected() {
    let parse_names = |buffer: &[u64]| {
        let mut names = Vec::new();
        parse(buffer, false, &mut |record| {
            names.push(OsString::from_wide(record.name));
            Ok(())
        })
        .map(|()| names)
    };
    let mut buffer = vec![0u64; BUFFER_WORDS];
    record(&mut buffer, 0, 112, ".");
    record(&mut buffer, 112, 120, "..");
    record(&mut buffer, 232, 0, "kept");
    assert_eq!(parse_names(&buffer).unwrap(), [OsString::from("kept")]);
    // A misaligned, overlapping or out-of-bounds next offset is invalid.
    for next in [113u32, 8, 70_000] {
        let mut buffer = vec![0u64; BUFFER_WORDS];
        record(&mut buffer, 0, next, "name-longer-than-eight");
        let error = parse_names(&buffer).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{next}");
    }
    // An odd byte length cannot be UTF-16.
    let mut buffer = vec![0u64; BUFFER_WORDS];
    record(&mut buffer, 0, 0, "odd");
    // SAFETY: the first record header was written above.
    unsafe { (*buffer.as_mut_ptr().cast::<FILE_ID_BOTH_DIR_INFO>()).FileNameLength = 3 };
    assert_eq!(
        parse_names(&buffer).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    // A name may not extend past the buffer.
    let mut buffer = vec![0u64; BUFFER_WORDS];
    record(&mut buffer, 0, 0, "x");
    // SAFETY: the first record header was written above.
    unsafe { (*buffer.as_mut_ptr().cast::<FILE_ID_BOTH_DIR_INFO>()).FileNameLength = 70_000 };
    assert_eq!(
        parse_names(&buffer).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn only_a_first_query_reports_an_empty_directory_as_no_such_file() {
    let not_found = Some(ERROR_FILE_NOT_FOUND as i32);
    let no_more = Some(ERROR_NO_MORE_FILES as i32);
    assert!(end_of_directory(not_found, false), "empty FAT/exFAT root");
    assert!(!end_of_directory(not_found, true), "a later query failed");
    assert!(end_of_directory(no_more, false) && end_of_directory(no_more, true));
    assert!(!end_of_directory(Some(5), false) && !end_of_directory(None, false));
}

#[test]
fn filetime_conversion_matches_std_metadata() {
    assert_eq!(system_time(116_444_736_000_000_000), Some(UNIX_EPOCH));
    assert_eq!(
        system_time(116_444_736_000_000_001),
        Some(UNIX_EPOCH + Duration::from_nanos(100))
    );
    assert_eq!(
        system_time(116_444_735_990_000_000),
        Some(UNIX_EPOCH - Duration::from_secs(1))
    );
    assert_eq!(
        system_time(0),
        Some(UNIX_EPOCH - Duration::from_secs(11_644_473_600))
    );
    assert_eq!(system_time(i64::MIN), None);
}
