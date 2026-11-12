use midnight_filesystem_api::{ErrorCode, Format};
use midnight_fs_archive::{Archive, Limits};
use sevenz_rust2::{
    ArchiveEntry, ArchiveWriter, EncoderConfiguration, EncoderMethod, SourceReader,
};
use std::io::Cursor;

fn sevenzip(solid: bool) -> Vec<u8> {
    sevenzip_named(solid, ["same.txt", "same.txt"])
}

fn sevenzip_named(solid: bool, names: [&str; 2]) -> Vec<u8> {
    let mut writer = ArchiveWriter::new(Cursor::new(Vec::new())).unwrap();
    writer.set_content_methods(vec![EncoderConfiguration::new(EncoderMethod::COPY)]);
    let data = [b"first".as_slice(), b"other".as_slice()];
    if solid {
        writer
            .push_archive_entries(
                names.into_iter().map(ArchiveEntry::new_file).collect(),
                data.into_iter()
                    .map(|bytes| SourceReader::new(Cursor::new(bytes)))
                    .collect(),
            )
            .unwrap();
    } else {
        for (name, bytes) in names.into_iter().zip(data) {
            writer
                .push_archive_entry(ArchiveEntry::new_file(name), Some(Cursor::new(bytes)))
                .unwrap();
        }
    }
    let bytes = writer.finish().unwrap().into_inner();
    assert_eq!(&bytes[32..42], b"firstother");
    bytes
}

#[test]
fn sevenzip_open_does_not_decode_members_and_only_requested_block_is_checked() {
    let mut bytes = sevenzip(false);
    bytes[37] ^= 1;
    let mut archive = Archive::open(Format::SevenZip, bytes, Limits::default())
        .expect("intact metadata must open without decoding the corrupt sibling block");
    let nodes = archive.list(1, 0).unwrap();
    assert_eq!(nodes.len(), 2);
    assert_ne!(nodes[0].id, nodes[1].id);
    assert_eq!(nodes[0].size, Some(5));
    assert_eq!(
        archive.read(nodes[0].id, 1, 3).unwrap(),
        (b"irs".to_vec(), false)
    );
    for _ in 0..2 {
        assert_eq!(
            archive.read(nodes[1].id, 0, 1).unwrap_err().code,
            ErrorCode::Corrupt,
            "no partially decoded cache may bypass a failed checksum"
        );
    }
    assert_eq!(archive.read(nodes[0].id, 0, 5).unwrap().0, b"first");
}

#[test]
fn sevenzip_solid_block_is_fully_verified_before_first_member_bytes_are_returned() {
    let mut bytes = sevenzip(true);
    bytes[37] ^= 1;
    let mut archive = Archive::open(Format::SevenZip, bytes, Limits::default())
        .expect("opening indexes solid metadata without decoding content");
    let nodes = archive.list(1, 0).unwrap();
    for node in &nodes {
        assert_eq!(
            archive.read(node.id, 0, 1).unwrap_err().code,
            ErrorCode::Corrupt
        );
    }
}

#[test]
fn sevenzip_metadata_budgets_are_checked_before_deferred_decoding() {
    let bytes = sevenzip(false);
    for limits in [
        Limits {
            max_entry: 4,
            ..Limits::default()
        },
        Limits {
            max_total: 9,
            ..Limits::default()
        },
        Limits {
            max_entries: 1,
            ..Limits::default()
        },
    ] {
        assert_eq!(
            Archive::open(Format::SevenZip, bytes.clone(), limits)
                .err()
                .unwrap()
                .code,
            ErrorCode::Limit
        );
    }
    assert_eq!(
        Archive::open(
            Format::SevenZip,
            sevenzip_named(true, ["../unsafe", "C:stream"]),
            Limits {
                max_total: 9,
                ..Limits::default()
            }
        )
        .err()
        .unwrap()
        .code,
        ErrorCode::Limit
    );
}

#[test]
fn sevenzip_reads_use_bounded_ranges_and_unsafe_siblings_are_not_published() {
    let mut archive = Archive::open(
        Format::SevenZip,
        sevenzip_named(true, ["same.txt", "../unsafe"]),
        Limits::default(),
    )
    .unwrap();
    let nodes = archive.list(1, 0).unwrap();
    assert!(!nodes[1].readable);
    assert_eq!(
        archive.read(nodes[1].id, 0, 1).unwrap_err().code,
        ErrorCode::Unsupported
    );
    assert_eq!(
        archive.read(nodes[0].id, 0, u32::MAX).unwrap_err().code,
        ErrorCode::Limit
    );
    assert_eq!(
        archive.read(nodes[0].id, 0, 0).unwrap_err().code,
        ErrorCode::Limit
    );
    assert_eq!(
        archive.read(nodes[0].id, u64::MAX, 1).unwrap(),
        (vec![], true)
    );
    assert_eq!(
        archive.read(nodes[0].id, 1, 256 * 1024).unwrap(),
        (b"irst".to_vec(), true)
    );
}

fn tar() -> Vec<u8> {
    let mut writer = tar::Builder::new(Vec::new());
    for data in [b"first".as_slice(), b"other".as_slice()] {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o600);
        header.set_cksum();
        writer.append_data(&mut header, "same.txt", data).unwrap();
    }
    writer.into_inner().unwrap()
}

#[test]
fn tar_ranges_preserve_duplicates_offsets_and_eof() {
    let mut archive = Archive::open(Format::Tar, tar(), Limits::default()).unwrap();
    let nodes = archive.list(1, 0).unwrap();
    assert_ne!(nodes[0].id, nodes[1].id);
    assert_eq!(nodes[0].size, Some(5));
    assert_eq!(
        archive.read(nodes[1].id, 2, 100).unwrap(),
        (b"her".to_vec(), true)
    );
    assert_eq!(archive.read(nodes[0].id, 0, 5).unwrap().0, b"first");
    assert_eq!(
        archive.read(nodes[0].id, u64::MAX, 1).unwrap(),
        (vec![], true)
    );
    assert_eq!(
        archive.read(nodes[0].id, 0, 0).unwrap_err().code,
        ErrorCode::Limit
    );
    assert_eq!(
        archive
            .read(nodes[0].id, 0, 256 * 1024 + 1)
            .unwrap_err()
            .code,
        ErrorCode::Limit
    );
}

#[test]
fn tar_seek_index_rejects_payload_truncation_and_bad_header_checksum() {
    let bytes = tar();
    for length in [512, 515, 1536, 1539] {
        assert!(
            Archive::open(Format::Tar, bytes[..length].to_vec(), Limits::default()).is_err(),
            "truncated to {length}"
        );
    }
    let mut corrupt = bytes;
    corrupt[0] ^= 1;
    assert_eq!(
        Archive::open(Format::Tar, corrupt, Limits::default())
            .err()
            .unwrap()
            .code,
        ErrorCode::Corrupt
    );
}
