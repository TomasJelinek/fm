//! Archives beyond the former worker budgets of 100,000 members and 8 MiB of names
//! open and list completely: the number of members and their names are unbudgeted.
use midnight_filesystem_api::{Format, MAX_BATCH, Node};
use midnight_fs_archive::{Archive, Limits};
use std::io::{self, Cursor};

#[path = "support/raw_sevenzip.rs"]
mod raw_sevenzip;

/// Empty members in this order: ZIP, TAR, 7z, and 7z with an uncompressed header.
fn fixtures(names: &[String]) -> [(&'static str, Format, Vec<u8>); 4] {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for name in names {
        zip.start_file(name.as_str(), options).unwrap();
    }
    let mut tar = tar::Builder::new(Vec::new());
    for name in names {
        let mut header = tar::Header::new_gnu();
        header.set_size(0);
        header.set_mode(0o644);
        tar.append_data(&mut header, name, io::empty()).unwrap();
    }
    let mut sevenzip = sevenz_rust2::ArchiveWriter::new(Cursor::new(Vec::new())).unwrap();
    for name in names {
        let entry = sevenz_rust2::ArchiveEntry::new_file(name);
        sevenzip.push_archive_entry::<&[u8]>(entry, None).unwrap();
    }
    [
        ("ZIP", Format::Zip, zip.finish().unwrap().into_inner()),
        ("TAR", Format::Tar, tar.into_inner().unwrap()),
        (
            "7z",
            Format::SevenZip,
            sevenzip.finish().unwrap().into_inner(),
        ),
        ("raw 7z", Format::SevenZip, raw_sevenzip::archive(names)),
    ]
}

/// Every child of `directory`, page by page as the host asks for them.
fn listed(archive: &Archive, directory: u64) -> Vec<Node> {
    let mut nodes: Vec<Node> = Vec::new();
    loop {
        let after = nodes.last().map_or(0, |node| node.id);
        let (page, more) = archive.page(directory, after).unwrap();
        assert!(page.len() <= MAX_BATCH);
        assert!(page.iter().all(|node| node.id > after));
        nodes.extend(page);
        if !more {
            return nodes;
        }
    }
}

#[test]
fn archives_of_more_than_100000_members_list_completely() {
    const MEMBERS: usize = 100_001;
    let names: Vec<String> = (0..MEMBERS)
        .map(|index| format!("big/{index:06}"))
        .collect();
    for (kind, format, bytes) in fixtures(&names) {
        let archive = Archive::open(format, bytes, Limits::default())
            .unwrap_or_else(|error| panic!("{kind}: {error}"));
        let root = listed(&archive, 1);
        assert_eq!(root.len(), 1, "{kind}");
        let members = listed(&archive, root[0].id);
        assert_eq!(members.len(), MEMBERS, "{kind}");
        for (index, member) in members.iter().enumerate() {
            assert_eq!(member.display, &names[index][4..], "{kind}");
            assert!(member.readable, "{kind}");
            assert_eq!(member.size, Some(0), "{kind}");
        }
    }
}

#[test]
fn archives_with_more_than_8_mib_of_names_list_completely() {
    // Each member's raw name and equal label hold 120,003 bytes: 100 of them hold
    // over 11 MiB. A name stays within the protocol's 64 KiB.
    let names: Vec<String> = (0..100)
        .map(|index| format!("{index:03}{}", "n".repeat(60_000)))
        .collect();
    for (kind, format, bytes) in fixtures(&names) {
        let archive = Archive::open(format, bytes, Limits::default())
            .unwrap_or_else(|error| panic!("{kind}: {error}"));
        let members = listed(&archive, 1);
        let listed: Vec<&str> = members.iter().map(|node| node.display.as_str()).collect();
        assert_eq!(listed, names, "{kind}");
        assert!(
            members
                .iter()
                .all(|node| node.name == node.display.as_bytes())
        );
    }
}
