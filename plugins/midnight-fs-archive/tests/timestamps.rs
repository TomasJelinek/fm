use midnight_filesystem_api::{Format, Kind, Timestamp};
use midnight_fs_archive::{Archive, Limits};
use std::io::{Cursor, Write};

fn utc(seconds: i64, nanoseconds: u32) -> Option<Timestamp> {
    Some(Timestamp::Utc {
        seconds,
        nanoseconds,
    })
}

fn zip_timestamp(fields: &[(u16, Vec<u8>)]) -> Option<Timestamp> {
    let mut options = zip::write::FullFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::from_date_and_time(2024, 2, 29, 12, 34, 56).unwrap());
    for (id, bytes) in fields {
        options.add_extra_data(*id, bytes, true).unwrap();
    }
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    writer.start_file("entry", options).unwrap();
    writer.write_all(b"data").unwrap();
    let archive = Archive::open(
        Format::Zip,
        writer.finish().unwrap().into_inner(),
        Limits::default(),
    )
    .unwrap();
    archive.list(1, 0).unwrap()[0].modified
}

fn extended(seconds: u32) -> (u16, Vec<u8>) {
    let mut bytes = vec![1];
    bytes.extend(seconds.to_le_bytes());
    (0x5455, bytes)
}
fn ntfs(ticks: u64) -> (u16, Vec<u8>) {
    let mut bytes = vec![0; 4];
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(24u16.to_le_bytes());
    bytes.extend(ticks.to_le_bytes());
    bytes.extend(0u64.to_le_bytes());
    bytes.extend(0u64.to_le_bytes());
    (0x000a, bytes)
}

#[test]
fn zip_preserves_timezone_free_dos_and_prefers_precise_utc() {
    assert_eq!(
        zip_timestamp(&[]),
        Some(Timestamp::Local {
            year: 2024,
            month: 2,
            day: 29,
            hour: 12,
            minute: 34,
            second: 56
        })
    );
    assert_eq!(
        zip_timestamp(&[extended(1_700_000_123)]),
        utc(1_700_000_123, 0)
    );
    let precise = 116_444_736_000_000_000 + 1_700_000_123 * 10_000_000 + 1_234_567;
    for fields in [
        vec![extended(1_700_000_124), ntfs(precise)],
        vec![ntfs(precise), extended(1_700_000_124)],
    ] {
        assert_eq!(zip_timestamp(&fields), utc(1_700_000_123, 123_456_700));
    }
    assert_eq!(
        zip_timestamp(&[ntfs(116_444_735_999_999_999)]),
        utc(-1, 999_999_900)
    );
}

fn append_tar(builder: &mut tar::Builder<Vec<u8>>, name: &str, mtime: u64, kind: tar::EntryType) {
    let mut header = tar::Header::new_ustar();
    header.set_size(0);
    header.set_mode(0o644);
    header.set_mtime(mtime);
    header.set_entry_type(kind);
    header.set_cksum();
    builder
        .append_data(&mut header, name, std::io::empty())
        .unwrap();
}
fn tar_timestamp(value: Option<&[u8]>) -> Option<Timestamp> {
    let mut builder = tar::Builder::new(Vec::new());
    if let Some(value) = value {
        builder.append_pax_extensions([("mtime", value)]).unwrap();
    }
    append_tar(&mut builder, "entry", 123, tar::EntryType::Regular);
    let archive = Archive::open(
        Format::Tar,
        builder.into_inner().unwrap(),
        Limits::default(),
    )
    .unwrap();
    archive.list(1, 0).unwrap()[0].modified
}

#[test]
fn tar_pax_override_preserves_negative_and_fractional_utc() {
    assert_eq!(tar_timestamp(None), utc(123, 0));
    for (text, expected) in [
        ("-0.25", utc(-1, 750_000_000)),
        ("-1.000000001", utc(-2, 999_999_999)),
        ("1.234567890", utc(1, 234_567_890)),
        ("+10.5", utc(10, 500_000_000)),
        ("-0.0000000001", utc(-1, 999_999_999)),
        ("0.0000000001", utc(0, 0)),
        ("-9223372036854775808", utc(i64::MIN, 0)),
        ("9223372036854775807.999999999", utc(i64::MAX, 999_999_999)),
    ] {
        assert_eq!(tar_timestamp(Some(text.as_bytes())), expected, "{text}");
    }
}

#[test]
fn malformed_explicit_pax_time_does_not_fall_back_to_placeholder_header() {
    for text in [
        "",
        "bad",
        "1e3",
        " 1",
        "1.",
        "1.2.3",
        "-9223372036854775808.1",
        "9223372036854775808",
    ] {
        assert_eq!(tar_timestamp(Some(text.as_bytes())), None, "{text}");
    }
    assert_eq!(tar_timestamp(Some(&[b'1'; 65])), None);
}

#[test]
fn explicit_directory_time_updates_the_existing_implicit_node_only() {
    let mut builder = tar::Builder::new(Vec::new());
    append_tar(
        &mut builder,
        "directory/entry",
        123,
        tar::EntryType::Regular,
    );
    builder
        .append_pax_extensions([("mtime", &b"456.125"[..])])
        .unwrap();
    append_tar(&mut builder, "directory/", 999, tar::EntryType::Directory);
    let archive = Archive::open(
        Format::Tar,
        builder.into_inner().unwrap(),
        Limits::default(),
    )
    .unwrap();
    let root = archive.list(1, 0).unwrap();
    assert_eq!(root.len(), 1);
    assert_eq!(root[0].kind, Kind::Directory);
    assert_eq!(root[0].modified, utc(456, 125_000_000));
    assert_eq!(
        archive.list(root[0].id, 0).unwrap()[0].modified,
        utc(123, 0)
    );
}

#[test]
fn sevenzip_records_keep_filetime_precision_and_epoch() {
    let mut writer = sevenz_rust2::ArchiveWriter::new(Cursor::new(Vec::new())).unwrap();
    let mut entry = sevenz_rust2::ArchiveEntry::new_file("entry");
    entry.has_last_modified_date = true;
    entry.last_modified_date = sevenz_rust2::NtTime::new(116_444_735_999_999_999);
    writer
        .push_archive_entry(entry, Some(Cursor::new(b"data")))
        .unwrap();
    let archive = Archive::open(
        Format::SevenZip,
        writer.finish().unwrap().into_inner(),
        Limits::default(),
    )
    .unwrap();
    assert_eq!(
        archive.list(1, 0).unwrap()[0].modified,
        utc(-1, 999_999_900)
    );
}

fn sevenzip_header(offset: u64, size: u64) -> Vec<u8> {
    let mut bytes = vec![0; 32];
    bytes[..6].copy_from_slice(&[0x37, 0x7a, 0xbc, 0xaf, 0x27, 0x1c]);
    bytes[7] = 4;
    bytes[12..20].copy_from_slice(&offset.to_le_bytes());
    bytes[20..28].copy_from_slice(&size.to_le_bytes());
    // Standard CRC32 for this owned fixture's start-header bytes.
    let mut crc = !0u32;
    for byte in &bytes[12..32] {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    bytes[8..12].copy_from_slice(&(!crc).to_le_bytes());
    bytes
}

/// A source of `length` bytes: `prefix`, then zeros that are never stored.
struct Sparse {
    prefix: Vec<u8>,
    length: u64,
    position: u64,
}
impl std::io::Read for Sparse {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let remaining = usize::try_from(self.length - self.position).unwrap_or(usize::MAX);
        let count = remaining.min(output.len());
        let output = &mut output[..count];
        output.fill(0);
        let start = usize::try_from(self.position)
            .unwrap_or(usize::MAX)
            .min(self.prefix.len());
        let copied = (self.prefix.len() - start).min(output.len());
        output[..copied].copy_from_slice(&self.prefix[start..start + copied]);
        self.position += output.len() as u64;
        Ok(output.len())
    }
}
impl std::io::Seek for Sparse {
    fn seek(&mut self, from: std::io::SeekFrom) -> std::io::Result<u64> {
        self.position = match from {
            std::io::SeekFrom::Start(position) => Some(position),
            std::io::SeekFrom::Current(delta) => self.position.checked_add_signed(delta),
            std::io::SeekFrom::End(delta) => self.length.checked_add_signed(delta),
        }
        .ok_or(std::io::ErrorKind::InvalidInput)?;
        Ok(self.position)
    }
}

#[test]
fn sevenzip_next_header_is_bounded_by_the_source_and_the_worker_memory() {
    use midnight_filesystem_api::ErrorCode;
    const MIB: u64 = 1024 * 1024;
    let open = |size: u64, length: u64| {
        let source = Sparse {
            prefix: sevenzip_header(0, size),
            length,
            position: 0,
        };
        Archive::open_stream(
            Format::SevenZip,
            Box::new(source),
            length,
            std::rc::Rc::new(midnight_fs_archive::storage::MemoryStorage::default()),
            Limits::default(),
        )
        .err()
        .unwrap()
    };
    // Beyond the former 64 MiB budget, as the uncompressed header of a million
    // members is: the parser reads it whole and checks its CRC (this one's is wrong).
    let read = open(64 * MIB + 1, 32 + 64 * MIB + 1);
    assert_eq!(read.code, ErrorCode::Corrupt);
    assert!(read.message.contains("NextHeaderCrcMismatch"), "{read}");
    // Beyond the source, it is refused unread.
    let outside = open(64 * MIB + 1, 32 + 64 * MIB);
    assert_eq!(outside.code, ErrorCode::Corrupt);
    assert!(outside.message.contains("range"), "{outside}");
    // Beyond an eighth of the worker's 1 GiB, a crafted header never reaches the
    // parser's allocation: the reason names both sizes.
    let crafted = open(4096 * MIB, 32 + 4096 * MIB);
    assert_eq!(crafted.code, ErrorCode::Limit);
    assert_eq!(
        crafted.message,
        "This 7z archive's uncompressed header of 4096 MiB needs more memory than the \
         archive worker has: it reads headers of up to 128 MiB"
    );
    assert_eq!(
        open(128 * MIB + 1, 32 + 128 * MIB + 1).code,
        ErrorCode::Limit
    );
}

#[test]
fn sevenzip_next_header_end_and_overflow_are_checked_before_parser_read() {
    for (offset, length) in [(u64::MAX, 1), (1, 0), (0, 1)] {
        let error = Archive::open(
            Format::SevenZip,
            sevenzip_header(offset, length),
            Limits::default(),
        )
        .err()
        .unwrap();
        assert_eq!(error.code, midnight_filesystem_api::ErrorCode::Corrupt);
        assert!(error.message.contains("range"), "{error}");
    }
}
