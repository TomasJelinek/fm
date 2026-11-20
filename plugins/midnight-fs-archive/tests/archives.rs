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
    // An embedder's declared-size budget; production keeps it unlimited.
    let limits = Limits {
        max_entry: 1,
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
fn a_file_that_is_not_a_zip_says_so_before_the_technical_reason() {
    for bytes in [b"plain text saved as notes.zip\n".to_vec(), Vec::new()] {
        let error = Archive::open(Format::Zip, bytes, Limits::default())
            .err()
            .unwrap();
        assert_eq!(error.code, ErrorCode::Corrupt);
        let (reason, details) = error.message.split_once('\n').unwrap();
        assert_eq!(reason, "This file is not a valid ZIP archive.");
        assert!(details.contains("EOCD"), "{details}");
    }
}

/// An archive member opened as an archive may be any file: each decoder's rejection
/// leads with the same plain sentence, then its own reason.
#[test]
fn files_that_are_no_archive_of_their_format_say_so_for_every_format() {
    for (format, kind) in [
        (Format::SevenZip, "7z archive"),
        (Format::Gzip, "gzip file"),
        (Format::TarGzip, "gzip-compressed TAR archive"),
        (Format::Xz, "XZ file"),
        (Format::Bzip2, "bzip2 file"),
        (Format::TarZstd, "Zstandard-compressed TAR archive"),
    ] {
        let error = Archive::open(
            format,
            b"plain text saved with an archive name\n".to_vec(),
            Limits::default(),
        )
        .err()
        .unwrap();
        assert_eq!(error.code, ErrorCode::Corrupt, "{format:?}: {error}");
        let (reason, details) = error.message.split_once('\n').unwrap();
        assert_eq!(reason, format!("This file is not a valid {kind}."));
        assert!(!details.is_empty(), "{format:?} keeps the decoder's reason");
    }
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

/// A stored ZIP whose central records may carry Info-ZIP Unicode Path fields (0x7075).
/// Each record is (written name, raw name of the same length, data, path fields), and
/// each path field is (the name its CRC-32 covers, path). The writer refuses duplicate
/// names and validates 0x7075 against an empty name, so raw names are patched in and
/// path fields are written under an unreserved tag, then retagged.
type UnicodeRecord<'a> = (&'a str, &'a str, &'a [u8], &'a [(&'a str, &'a str)]);
fn unicode_path_zip(records: &[UnicodeRecord<'_>]) -> Vec<u8> {
    let find = |source: &[u8], needle: &[u8]| {
        source
            .windows(needle.len())
            .position(|window| window == needle)
    };
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let mut fields = Vec::new();
    for (written, _, data, paths) in records {
        let mut options = zip::write::FullFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (covers, path) in *paths {
            let mut crc = flate2::Crc::new();
            crc.update(covers.as_bytes());
            let mut body = vec![1];
            body.extend(crc.sum().to_le_bytes());
            body.extend(path.as_bytes());
            let mut field = 0x1234u16.to_le_bytes().to_vec();
            field.extend((body.len() as u16).to_le_bytes());
            field.extend(&body);
            fields.push(field);
            options.add_extra_data(0x1234, body, true).unwrap();
        }
        writer.start_file(*written, options).unwrap();
        writer.write_all(data).unwrap();
    }
    let mut source = writer.finish().unwrap().into_inner();
    for field in fields {
        let at = find(&source, &field).unwrap();
        source[at..at + 2].copy_from_slice(&0x7075u16.to_le_bytes());
    }
    for (written, raw, _, _) in records {
        let offsets: Vec<_> = source
            .windows(written.len())
            .enumerate()
            .filter_map(|(index, name)| (name == written.as_bytes()).then_some(index))
            .collect();
        for at in offsets {
            source[at..at + raw.len()].copy_from_slice(raw.as_bytes());
        }
    }
    source
}

#[test]
fn zip_records_merged_through_a_unicode_path_are_listed_but_unreadable() {
    // Different raw names, but the third record's Unicode Path (which 7-Zip and
    // Info-ZIP also display) makes the reader index it as `setup.exe`.
    let records: [UnicodeRecord<'_>; 3] = [
        ("setup.exe", "setup.exe", b"benign", &[]),
        ("readme.txt", "readme.txt", b"read me", &[]),
        (
            "setup.exf",
            "setup.exf",
            b"other!",
            &[("setup.exf", "setup.exe")],
        ),
    ];
    let source = unicode_path_zip(&records);
    let mut archive = Archive::open(Format::Zip, source, Limits::default()).unwrap();
    let nodes = archive.list(1, 0).unwrap();
    let names: Vec<_> = nodes.iter().map(|node| node.name.as_slice()).collect();
    assert_eq!(names, [b"setup.exe".as_slice(), b"readme.txt"]);
    assert!(!nodes[0].readable, "two records merged under setup.exe");
    assert_eq!(
        archive.read(nodes[0].id, 0, 16).unwrap_err().code,
        ErrorCode::Unsupported
    );
    assert_eq!(archive.read(nodes[1].id, 0, 16).unwrap().0, b"read me");
}

#[test]
fn zip_duplicates_are_judged_by_the_names_the_reader_indexes() {
    let records: [UnicodeRecord<'_>; 4] = [
        // One raw name with two different Unicode Paths: two separate members.
        ("a.tx1", "a.txt", b"one", &[("a.txt", "alpha.txt")]),
        ("a.tx2", "a.txt", b"two", &[("a.txt", "beta.txt")]),
        // Chained paths: the second covers the first one's result and merges with `beta.txt`.
        (
            "c.txt",
            "c.txt",
            b"three",
            &[("c.txt", "gamma.txt"), ("gamma.txt", "beta.txt")],
        ),
        // The same raw name without a path is yet another member.
        ("a.tx3", "a.txt", b"four", &[]),
    ];
    let mut archive =
        Archive::open(Format::Zip, unicode_path_zip(&records), Limits::default()).unwrap();
    let nodes = archive.list(1, 0).unwrap();
    let listed: Vec<_> = nodes
        .iter()
        .map(|node| (node.name.as_slice(), node.readable))
        .collect();
    assert_eq!(
        listed,
        [
            (b"alpha.txt".as_slice(), true),
            (b"beta.txt", false),
            (b"a.txt", true)
        ]
    );
    assert_eq!(archive.read(nodes[0].id, 0, 8).unwrap().0, b"one");
    assert_eq!(archive.read(nodes[2].id, 0, 8).unwrap().0, b"four");
    // The record walk relies on the reader refusing a path whose CRC-32 does not
    // cover the name it replaces; such a field never reaches the walk.
    let mismatched: [UnicodeRecord<'_>; 1] = [("b.txt", "b.txt", b"x", &[("other", "path.txt")])];
    assert_eq!(
        Archive::open(
            Format::Zip,
            unicode_path_zip(&mismatched),
            Limits::default()
        )
        .err()
        .unwrap()
        .code,
        ErrorCode::Corrupt
    );
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

/// A source that fails like a dropped share or a removed drive once `remaining` more
/// bytes were read from it.
struct Failing {
    inner: Cursor<Vec<u8>>,
    remaining: std::rc::Rc<std::cell::Cell<u64>>,
}
impl std::io::Read for Failing {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let remaining = self.remaining.get();
        if remaining == 0 {
            return Err(std::io::Error::other("Input/output error"));
        }
        let limit = usize::try_from(remaining)
            .unwrap_or(usize::MAX)
            .min(output.len());
        let count = std::io::Read::read(&mut self.inner, &mut output[..limit])?;
        self.remaining.set(remaining - count as u64);
        Ok(count)
    }
}
impl std::io::Seek for Failing {
    fn seek(&mut self, from: std::io::SeekFrom) -> std::io::Result<u64> {
        std::io::Seek::seek(&mut self.inner, from)
    }
}

fn open_failing(
    format: Format,
    bytes: Vec<u8>,
    remaining: u64,
) -> (
    Result<Archive, midnight_fs_archive::Error>,
    std::rc::Rc<std::cell::Cell<u64>>,
) {
    let remaining = std::rc::Rc::new(std::cell::Cell::new(remaining));
    let length = bytes.len() as u64;
    let source = Failing {
        inner: Cursor::new(bytes),
        remaining: remaining.clone(),
    };
    let opened = Archive::open_stream(
        format,
        Box::new(source),
        length,
        std::rc::Rc::new(midnight_fs_archive::storage::MemoryStorage::default()),
        Limits::default(),
    );
    (opened, remaining)
}

/// Incompressible bytes, so that every format spans several source reads.
fn noise(length: usize) -> Vec<u8> {
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    (0..length)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect()
}

fn assert_source_failure(error: &midnight_fs_archive::Error, context: &str) {
    assert_eq!(error.code, ErrorCode::Io, "{context}: {error}");
    assert_eq!(error.message, "Input/output error", "{context}");
}

#[test]
fn a_failed_source_read_is_an_io_failure_never_an_invalid_archive() {
    let data = noise(2_000_000);
    let mut tar = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_size(data.len() as u64);
    header.set_mode(0o644);
    tar.append_data(&mut header, "data.bin", data.as_slice())
        .unwrap();
    let tar = tar.into_inner().unwrap();
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gzip.write_all(&tar).unwrap();
    let tar_gz = gzip.finish().unwrap();
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("data.bin", stored).unwrap();
    zip.write_all(&data).unwrap();
    let zip = zip.finish().unwrap().into_inner();
    let mut sevenzip = sevenz_rust2::ArchiveWriter::new(Cursor::new(Vec::new())).unwrap();
    sevenzip.set_content_methods(vec![sevenz_rust2::EncoderConfiguration::new(
        sevenz_rust2::EncoderMethod::COPY,
    )]);
    sevenzip
        .push_archive_entry(
            sevenz_rust2::ArchiveEntry::new_file("data.bin"),
            Some(data.as_slice()),
        )
        .unwrap();
    let sevenzip = sevenzip.finish().unwrap().into_inner();
    // Opening fails at the first read, or halfway through a compressed TAR. Every
    // decoder wraps or reports the failure its own way.
    let half = tar_gz.len() as u64 / 2;
    for (format, bytes, remaining) in [
        (Format::TarGzip, tar_gz.clone(), 0),
        (Format::TarGzip, tar_gz.clone(), half),
        (Format::Gzip, tar_gz.clone(), 0),
        (Format::Zip, zip.clone(), 0),
        (Format::Tar, tar.clone(), 0),
        (Format::Xz, tar_gz.clone(), 0),
        (Format::SevenZip, sevenzip.clone(), 0),
    ] {
        let context = format!("opening {format:?} after {remaining} bytes");
        let (opened, _) = open_failing(format, bytes, remaining);
        assert_source_failure(&opened.err().unwrap(), &context);
    }
    // A member read that fails after the archive opened, while it is staged.
    for (format, bytes) in [
        (Format::Zip, zip),
        (Format::Tar, tar),
        (Format::SevenZip, sevenzip),
    ] {
        let context = format!("reading a {format:?} member");
        let (opened, remaining) = open_failing(format, bytes, u64::MAX);
        let mut archive = opened.unwrap_or_else(|error| panic!("{context}: {error}"));
        let member = archive.list(1, 0).unwrap()[0].id;
        remaining.set(0);
        assert_source_failure(&archive.read(member, 0, 4).unwrap_err(), &context);
        // The archive still serves the member once its source reads again.
        remaining.set(u64::MAX);
        assert_eq!(
            archive.read(member, 0, 4).unwrap().0,
            data[..4],
            "{context}"
        );
    }
}
