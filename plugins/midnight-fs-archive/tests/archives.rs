use midnight_filesystem_api::{ErrorCode, Format, Kind};
use midnight_fs_archive::{Archive, Limits};
use std::io::{Cursor, Write};

fn zip_fixture() -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    writer.start_file("日本語/čau.txt", options).unwrap();
    writer.write_all(b"hello").unwrap();
    writer.start_file("../escape.txt", options).unwrap();
    writer.write_all(b"unsafe").unwrap();
    writer.start_file("name:stream", options).unwrap();
    writer.write_all(b"unsafe").unwrap();
    writer.finish().unwrap().into_inner()
}

#[test]
fn zip_lists_implicit_directories_and_reads_members_by_identity() {
    let mut archive = Archive::open(Format::Zip, zip_fixture(), Limits::default()).unwrap();
    let root = archive.list(1, 0).unwrap();
    let folder = root.iter().find(|n| n.display == "日本語").unwrap();
    assert_eq!(folder.kind, Kind::Directory);
    let files = archive.list(folder.id, 0).unwrap();
    assert_eq!(files[0].display, "čau.txt");
    assert_eq!(archive.read(files[0].id, 1, 3).unwrap().0, b"ell");
    assert_eq!(archive.read(files[0].id, 5, 10).unwrap(), (vec![], true));
}

#[test]
fn traversal_and_ads_are_visible_but_never_readable() {
    let mut archive = Archive::open(Format::Zip, zip_fixture(), Limits::default()).unwrap();
    for node in archive
        .list(1, 0)
        .unwrap()
        .iter()
        .filter(|n| n.kind == Kind::File)
    {
        assert!(!node.readable);
        assert_eq!(
            archive.read(node.id, 0, 8).unwrap_err().code,
            ErrorCode::Unsupported
        );
    }
}

#[test]
fn corrupt_archives_and_resource_limits_are_explicit_failures() {
    assert!(Archive::open(Format::Zip, b"bad zip".to_vec(), Limits::default()).is_err());
    let limits = Limits {
        max_entries: 1,
        ..Limits::default()
    };
    assert_eq!(
        Archive::open(Format::Zip, zip_fixture(), limits)
            .err()
            .unwrap()
            .code,
        ErrorCode::Limit
    );
}

#[test]
fn zip_crc_failure_rejects_the_mount_before_publishing_member_bytes() {
    let mut source = zip_fixture();
    let at = source
        .windows(5)
        .position(|bytes| bytes == b"hello")
        .unwrap();
    source[at] ^= 1;
    assert!(Archive::open(Format::Zip, source, Limits::default()).is_err());
}

#[test]
fn encrypted_zip_members_are_explicitly_unreadable_without_password_guessing() {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file(
            "secret",
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored),
        )
        .unwrap();
    writer.write_all(b"not an encryption payload").unwrap();
    let mut source = writer.finish().unwrap().into_inner();
    source[6] |= 1;
    let central = source.windows(4).position(|b| b == b"PK\x01\x02").unwrap();
    source[central + 8] |= 1;
    let mut archive = Archive::open(Format::Zip, source, Limits::default()).unwrap();
    let node = archive.list(1, 0).unwrap().remove(0);
    assert!(!node.readable);
    assert_eq!(
        archive.read(node.id, 0, 1).unwrap_err().code,
        ErrorCode::Unsupported
    );
}

#[test]
fn raw_non_unicode_zip_names_remain_provider_identity_bytes() {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file(
            "raw.txt",
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored),
        )
        .unwrap();
    writer.write_all(b"hello").unwrap();
    let mut source = writer.finish().unwrap().into_inner();
    let offsets: Vec<_> = source
        .windows(7)
        .enumerate()
        .filter_map(|(i, b)| (b == b"raw.txt").then_some(i))
        .collect();
    for at in offsets {
        source[at] = 255;
    }
    let mut archive = Archive::open(Format::Zip, source, Limits::default()).unwrap();
    let node = archive.list(1, 0).unwrap().remove(0);
    assert_eq!(node.name, b"\xffaw.txt");
    assert_eq!(archive.read(node.id, 0, 5).unwrap().0, b"hello");
}

#[test]
fn cp437_implicit_directory_preserves_decoded_display_and_raw_identity() {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file(
            "cafe/file.txt",
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored),
        )
        .unwrap();
    writer.write_all(b"hello").unwrap();
    let mut source = writer.finish().unwrap().into_inner();
    let offsets: Vec<_> = source
        .windows(13)
        .enumerate()
        .filter_map(|(index, name)| (name == b"cafe/file.txt").then_some(index))
        .collect();
    for offset in offsets {
        source[offset + 3] = 0x82;
    }
    let archive = Archive::open(Format::Zip, source, Limits::default()).unwrap();
    let directory = archive.list(1, 0).unwrap().remove(0);
    assert_eq!(directory.name, b"caf\x82");
    assert_eq!(directory.display, "café");
    assert_eq!(
        archive.list(directory.id, 0).unwrap()[0].display,
        "file.txt"
    );
}
