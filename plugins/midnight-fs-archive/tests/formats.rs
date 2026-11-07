use midnight_filesystem_api::{ErrorCode, Format, Kind};
use midnight_fs_archive::{Archive, Limits};
use std::io::{Cursor, Write};

fn tar_fixture() -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_size(5);
    header.set_mode(0o644);
    header.set_cksum();
    builder
        .append_data(
            &mut header,
            format!("{}日本語.txt", "long/".repeat(60)),
            &b"hello"[..],
        )
        .unwrap();
    let mut link = tar::Header::new_gnu();
    link.set_entry_type(tar::EntryType::Symlink);
    link.set_size(0);
    link.set_mode(0o777);
    link.set_cksum();
    builder
        .append_link(&mut link, "link", "../../secret")
        .unwrap();
    builder.into_inner().unwrap()
}
fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut writer = flate2::GzBuilder::new()
        .filename("../untrusted-name")
        .write(Vec::new(), flate2::Compression::default());
    writer.write_all(bytes).unwrap();
    writer.finish().unwrap()
}
#[test]
fn tar_gnu_long_names_and_links_are_preserved_without_following() {
    let mut archive = Archive::open(Format::Tar, tar_fixture(), Limits::default()).unwrap();
    let link = archive
        .list(1, 0)
        .unwrap()
        .into_iter()
        .find(|n| n.kind == Kind::Link)
        .unwrap();
    assert!(!link.readable);
    assert_eq!(
        archive.read(link.id, 0, 16).unwrap_err().code,
        ErrorCode::Unsupported
    );
    let mut directory = 1;
    for _ in 0..60 {
        directory = archive
            .list(directory, 0)
            .unwrap()
            .into_iter()
            .find(|n| n.kind == Kind::Directory)
            .unwrap()
            .id;
    }
    let file = archive.list(directory, 0).unwrap().remove(0);
    assert_eq!(file.display, "日本語.txt");
    assert_eq!(archive.read(file.id, 0, 16).unwrap().0, b"hello");
}
#[test]
fn gzip_multimember_ignores_untrusted_fname_and_validates_trailer() {
    let mut bytes = gzip(b"hello");
    bytes.extend(gzip(b" world"));
    let mut archive = Archive::open(Format::Gzip, bytes.clone(), Limits::default()).unwrap();
    let node = archive.list(1, 0).unwrap().remove(0);
    assert_eq!(node.name, b"content");
    assert_eq!(archive.read(node.id, 0, 100).unwrap().0, b"hello world");
    let end = bytes.len();
    bytes[end - 5] ^= 1;
    assert!(Archive::open(Format::Gzip, bytes, Limits::default()).is_err());
}
#[test]
fn tar_gzip_is_a_standalone_format_and_truncated_inputs_fail() {
    let bytes = gzip(&tar_fixture());
    let archive = Archive::open(Format::TarGzip, bytes.clone(), Limits::default()).unwrap();
    assert_eq!(archive.list(1, 0).unwrap().len(), 2);
    assert!(
        Archive::open(
            Format::TarGzip,
            bytes[..bytes.len() - 3].to_vec(),
            Limits::default()
        )
        .is_err()
    );
}
#[test]
fn sevenzip_solid_members_are_read_once_then_addressed_independently() {
    let mut writer = sevenz_rust2::ArchiveWriter::new(Cursor::new(Vec::new())).unwrap();
    writer
        .push_archive_entries(
            vec![
                sevenz_rust2::ArchiveEntry::new_file("a.txt"),
                sevenz_rust2::ArchiveEntry::new_file("a.txt"),
            ],
            vec![
                sevenz_rust2::SourceReader::new(Cursor::new(b"first")),
                sevenz_rust2::SourceReader::new(Cursor::new(b"other")),
            ],
        )
        .unwrap();
    let bytes = writer.finish().unwrap().into_inner();
    let mut archive = Archive::open(Format::SevenZip, bytes.clone(), Limits::default()).unwrap();
    let entries = archive.list(1, 0).unwrap();
    assert_eq!(entries.len(), 2);
    assert_ne!(entries[0].id, entries[1].id);
    assert_eq!(archive.read(entries[1].id, 0, 5).unwrap().0, b"other");
    assert_eq!(archive.read(entries[0].id, 0, 5).unwrap().0, b"first");
    assert!(Archive::open(Format::SevenZip, bytes[..20].to_vec(), Limits::default()).is_err());
}
#[test]
fn high_expansion_ratio_is_rejected_before_publication() {
    let bytes = gzip(&vec![0; 128 * 1024]);
    assert_eq!(
        Archive::open(Format::Gzip, bytes, Limits::default())
            .err()
            .unwrap()
            .code,
        ErrorCode::Limit
    );
}
