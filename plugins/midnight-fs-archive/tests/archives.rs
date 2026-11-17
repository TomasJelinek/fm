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
fn zip_crc_failure_rejects_first_read_before_publishing_any_member_bytes() {
    let mut source = zip_fixture();
    let at = source
        .windows(5)
        .position(|bytes| bytes == b"hello")
        .unwrap();
    source[at] ^= 1;
    let mut archive = Archive::open(Format::Zip, source, Limits::default()).unwrap();
    let folder = archive
        .list(1, 0)
        .unwrap()
        .into_iter()
        .find(|n| n.kind == Kind::Directory)
        .unwrap();
    let file = archive.list(folder.id, 0).unwrap().remove(0);
    assert!(archive.read(file.id, 0, 1).is_err());
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
fn zip_records_merged_by_duplicate_raw_names_are_listed_but_unreadable() {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, data) in [
        ("setup.exe", b"benign".as_slice()),
        ("readme.txt", b"read me"),
        ("setup.exf", b"other!"),
    ] {
        writer.start_file(name, options).unwrap();
        writer.write_all(data).unwrap();
    }
    let mut source = writer.finish().unwrap().into_inner();
    // The writer refuses duplicates; rename the third record's local and central name.
    let offsets: Vec<_> = source
        .windows(9)
        .enumerate()
        .filter_map(|(index, name)| (name == b"setup.exf").then_some(index))
        .collect();
    assert_eq!(offsets.len(), 2);
    for offset in offsets {
        source[offset + 8] = b'e';
    }
    let mut archive = Archive::open(Format::Zip, source, Limits::default()).unwrap();
    let nodes = archive.list(1, 0).unwrap();
    let names: Vec<_> = nodes.iter().map(|node| node.name.as_slice()).collect();
    assert_eq!(names, [b"setup.exe".as_slice(), b"readme.txt"]);
    assert!(
        !nodes[0].readable,
        "an ambiguous ZIP member must not be readable"
    );
    assert_eq!(
        archive.read(nodes[0].id, 0, 16).unwrap_err().code,
        ErrorCode::Unsupported
    );
    assert_eq!(archive.read(nodes[1].id, 0, 16).unwrap().0, b"read me");
}

fn raw_tar_entry(
    builder: &mut tar::Builder<Vec<u8>>,
    name: &[u8],
    kind: tar::EntryType,
    data: &[u8],
) {
    let mut header = tar::Header::new_gnu();
    header.as_old_mut().name[..name.len()].copy_from_slice(name);
    header.set_entry_type(kind);
    header.set_size(data.len() as u64);
    header.set_mode(0o644);
    header.set_mtime(1_700_000_000);
    header.set_cksum();
    builder.append(&header, data).unwrap();
}

#[test]
fn dot_prefixed_tar_members_form_an_ordinary_readable_tree() {
    let mut builder = tar::Builder::new(Vec::new());
    raw_tar_entry(&mut builder, b"./", tar::EntryType::Directory, b"");
    raw_tar_entry(&mut builder, b"./usr/", tar::EntryType::Directory, b"");
    raw_tar_entry(
        &mut builder,
        b"./usr/./bin/tool",
        tar::EntryType::Regular,
        b"data",
    );
    raw_tar_entry(&mut builder, b"./usr/.", tar::EntryType::Directory, b"");
    raw_tar_entry(
        &mut builder,
        b"./usr/bin/.",
        tar::EntryType::Regular,
        b"bad",
    );
    let mut archive = Archive::open(
        Format::Tar,
        builder.into_inner().unwrap(),
        Limits::default(),
    )
    .unwrap();
    let root = archive.list(1, 0).unwrap();
    assert_eq!(root.len(), 2, "{root:?}");
    let usr = &root[0];
    assert_eq!(
        (usr.name.as_slice(), usr.display.as_str()),
        (b"usr".as_slice(), "usr")
    );
    assert_eq!(usr.kind, Kind::Directory);
    assert!(
        usr.modified.is_some(),
        "the `./usr/.` record names `usr` itself"
    );
    // A file cannot be named by a final `.`: it stays a visible, unreadable root record.
    assert_eq!(root[1].name, b"./usr/bin/.");
    assert!(!root[1].readable);
    let bin = archive.list(usr.id, 0).unwrap().remove(0);
    assert_eq!(
        (bin.name.as_slice(), bin.kind),
        (b"bin".as_slice(), Kind::Directory)
    );
    let tool = archive.list(bin.id, 0).unwrap().remove(0);
    assert_eq!(tool.name, b"tool");
    assert!(tool.readable);
    assert_eq!(archive.read(tool.id, 0, 16).unwrap().0, b"data");
}

#[test]
fn unsafe_directory_records_are_visible_but_not_navigable() {
    let mut builder = tar::Builder::new(Vec::new());
    for name in [b"../".as_slice(), b"C:/", b"/abs/"] {
        raw_tar_entry(&mut builder, name, tar::EntryType::Directory, b"");
    }
    let archive = Archive::open(
        Format::Tar,
        builder.into_inner().unwrap(),
        Limits::default(),
    )
    .unwrap();
    let root = archive.list(1, 0).unwrap();
    assert_eq!(root.len(), 3);
    for node in root {
        assert_eq!(node.kind, Kind::Other, "{:?}", node.name);
        assert!(!node.readable);
        assert_eq!(
            archive.list(node.id, 0).unwrap_err().code,
            ErrorCode::NotFound
        );
    }
}

#[test]
fn pax_global_headers_are_not_members_and_huge_metadata_records_fail_cleanly() {
    let mut builder = tar::Builder::new(Vec::new());
    let comment = b"52 comment=8d1cf6c0b3e3a0c4d2b3a4f5e6d7c8b9a0b1c2d3\n";
    raw_tar_entry(
        &mut builder,
        b"pax_global_header",
        tar::EntryType::XGlobalHeader,
        comment,
    );
    raw_tar_entry(&mut builder, b"file.txt", tar::EntryType::Regular, b"text");
    let bytes = builder.into_inner().unwrap();
    let archive = Archive::open(Format::Tar, bytes.clone(), Limits::default()).unwrap();
    let names: Vec<_> = archive
        .list(1, 0)
        .unwrap()
        .into_iter()
        .map(|node| node.name)
        .collect();
    assert_eq!(names, [b"file.txt".to_vec()]);
    // A long-name record the tar crate would buffer completely fails as corrupt
    // metadata instead of exhausting worker memory.
    let mut builder = tar::Builder::new(Vec::new());
    raw_tar_entry(
        &mut builder,
        b"././@LongLink",
        tar::EntryType::GNULongName,
        &vec![b'a'; 17 * 1024 * 1024],
    );
    raw_tar_entry(&mut builder, b"file.txt", tar::EntryType::Regular, b"text");
    assert_eq!(
        Archive::open(
            Format::Tar,
            builder.into_inner().unwrap(),
            Limits::default()
        )
        .err()
        .unwrap()
        .code,
        ErrorCode::Corrupt
    );
}

#[test]
fn listing_pages_report_more_only_when_children_remain() {
    for count in [128usize, 130] {
        let mut builder = tar::Builder::new(Vec::new());
        for index in 0..count {
            raw_tar_entry(
                &mut builder,
                format!("f{index:03}").as_bytes(),
                tar::EntryType::Regular,
                b"",
            );
        }
        let archive = Archive::open(
            Format::Tar,
            builder.into_inner().unwrap(),
            Limits::default(),
        )
        .unwrap();
        let (first, more) = archive.page(1, 0).unwrap();
        assert_eq!((first.len(), more), (128, count > 128));
        let (rest, more) = archive.page(1, first[127].id).unwrap();
        assert_eq!((rest.len(), more), (count - 128, false));
        assert_eq!(archive.list(1, 0).unwrap(), first);
    }
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
