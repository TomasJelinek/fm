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

#[test]
fn sevenzip_next_header_budget_is_checked_before_parser_allocation() {
    let error = Archive::open(
        Format::SevenZip,
        sevenzip_header(0, 64 * 1024 * 1024 + 1),
        Limits::default(),
    )
    .err()
    .unwrap();
    assert_eq!(error.code, midnight_filesystem_api::ErrorCode::Limit);
    assert!(error.message.contains("metadata"), "{error}");
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
