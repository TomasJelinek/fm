//! Check the pinned decoder's block-only integrity path, independently of member CRCs.
use super::*;
use sevenz_rust2::{ArchiveWriter, EncoderConfiguration, SourceReader};
use std::io::Cursor;

fn solid_of(members: &[(&str, &[u8])]) -> Archive {
    let mut writer = ArchiveWriter::new(Cursor::new(Vec::new())).unwrap();
    writer.set_content_methods(vec![EncoderConfiguration::new(EncoderMethod::COPY)]);
    writer
        .push_archive_entries(
            members
                .iter()
                .map(|(name, _)| ArchiveEntry::new_file(name))
                .collect(),
            members
                .iter()
                .map(|(_, bytes)| SourceReader::new(Cursor::new(bytes.to_vec())))
                .collect(),
        )
        .unwrap();
    Archive::open(
        Format::SevenZip,
        writer.finish().unwrap().into_inner(),
        Limits::default(),
    )
    .unwrap()
}

fn solid() -> Archive {
    solid_of(&[("a", b"first"), ("b", b"other")])
}

fn empty_source() -> Reader {
    Reader::new(Box::new(Cursor::new(Vec::new())), 0)
}

#[test]
fn a_failed_decode_of_the_same_block_keeps_its_verified_siblings() {
    let mut archive = solid_of(&[("a", b"first"), ("b", b"other"), ("c", b"third")]);
    assert_eq!(archive.read(2, 0, 5).unwrap().0, b"first");
    let storage = archive.storage.clone();
    let content = archive.sevenzip_content.as_mut().unwrap();
    content.source = empty_source();
    // Decoding the block again (for example after the host evicted the lease of
    // `a`, or a cancelled preview) fails here, but must not discard `b` and `c`.
    assert_eq!(
        content.decode(2, storage).unwrap_err().code,
        ErrorCode::Corrupt
    );
    assert_eq!(archive.read(4, 0, 5).unwrap().0, b"third");
    assert_eq!(archive.read(3, 0, 5).unwrap().0, b"other");
}

#[test]
fn block_only_crc_is_verified_before_even_an_early_member_is_published() {
    for corrupt in [false, true] {
        let mut archive = solid();
        let content = archive.sevenzip_content.as_mut().unwrap();
        for file in &mut content.metadata.files {
            file.has_crc = false;
        }
        // Writer fixtures carry member CRCs. Exercise the alternate legal index form:
        // only one block CRC, covering the ten ASCII bytes "firstother".
        content.metadata.blocks[0].has_crc = true;
        content.metadata.blocks[0].crc = 0x738f_fa0f;
        if corrupt {
            content.metadata.blocks[0].crc ^= 1;
            assert_eq!(archive.read(2, 0, 1).unwrap_err().code, ErrorCode::Corrupt);
            assert!(archive.contents.is_empty());
            let content = archive.sevenzip_content.as_ref().unwrap();
            assert!(
                content.retained.members.is_empty(),
                "an unverified sibling must never be retained"
            );
        } else {
            assert_eq!(archive.read(2, 0, 1).unwrap().0, b"f");
        }
    }
}

#[test]
fn short_decode_is_rejected_even_when_no_checksum_is_present() {
    let mut archive = solid();
    let content = archive.sevenzip_content.as_mut().unwrap();
    for file in &mut content.metadata.files {
        file.has_crc = false;
    }
    content.metadata.blocks[0].has_crc = false;
    content.source = empty_source();
    assert_eq!(archive.read(2, 0, 1).unwrap_err().code, ErrorCode::Corrupt);
    assert!(archive.contents.is_empty());
}

#[test]
fn verified_solid_siblings_are_served_without_another_block_pass() {
    let mut archive = solid();
    assert_eq!(archive.read(2, 0, 1).unwrap().0, b"f");
    // The sibling was verified with the block: no source bytes are needed for it.
    archive.sevenzip_content.as_mut().unwrap().source = empty_source();
    assert_eq!(archive.read(3, 0, 5).unwrap().0, b"other");
    let content = archive.sevenzip_content.as_ref().unwrap();
    assert!(content.retained.members.is_empty());
    assert_eq!(content.retained.cost, 0);
    assert_eq!(archive.read(2, 0, 5).unwrap().0, b"first");
}

#[test]
fn retention_prefers_later_siblings_within_its_window() {
    let mut writer = ArchiveWriter::new(Cursor::new(Vec::new())).unwrap();
    writer.set_content_methods(vec![EncoderConfiguration::new(EncoderMethod::COPY)]);
    let names = ["0", "1", "2", "3", "4"];
    writer
        .push_archive_entries(
            names
                .iter()
                .map(|name| ArchiveEntry::new_file(name))
                .collect(),
            names
                .iter()
                .map(|name| SourceReader::new(Cursor::new(name.repeat(10).into_bytes())))
                .collect(),
        )
        .unwrap();
    let archive = Archive::open(
        Format::SevenZip,
        writer.finish().unwrap().into_inner(),
        Limits::default(),
    )
    .unwrap();
    let mut content = archive.sevenzip_content.unwrap();
    let planned = |content: &Content, requested| {
        let mut planned: Vec<_> = content.retention(0, requested).into_keys().collect();
        planned.sort();
        planned
    };
    assert_eq!(planned(&content, 2), [0, 1, 3, 4]);
    // Members of a large block: the window holds exactly two siblings, later ones first.
    for file in &mut content.metadata.files {
        file.size = RETAINED_SIBLINGS / 2 - RETAINED_ENTRY_COST;
    }
    assert_eq!(planned(&content, 2), [3, 4]);
    // Copying out from the first member keeps the next two; the fourth decodes again.
    assert_eq!(planned(&content, 0), [1, 2]);
    // Siblings still retained from an earlier decode count against the window.
    content.retained.cost = RETAINED_SIBLINGS / 2;
    assert_eq!(planned(&content, 0), [1]);
    content.retained.cost = 0;
    for file in &mut content.metadata.files {
        file.size = RETAINED_SIBLINGS;
    }
    assert!(content.retention(0, 2).is_empty());
}

#[test]
fn an_undecodable_block_only_makes_its_own_members_unreadable() {
    let mut writer = ArchiveWriter::new(Cursor::new(Vec::new())).unwrap();
    writer.set_content_methods(vec![EncoderConfiguration::new(EncoderMethod::COPY)]);
    for (name, bytes) in [("a", b"first"), ("b", b"other")] {
        writer
            .push_archive_entry(ArchiveEntry::new_file(name), Some(Cursor::new(bytes)))
            .unwrap();
    }
    let bytes = writer.finish().unwrap().into_inner();
    let length = bytes.len() as u64;
    let mut source = Reader::new(Box::new(Cursor::new(bytes)), length);
    let mut metadata = sevenz_rust2::Archive::read(&mut source, &Password::empty()).unwrap();
    assert_eq!(metadata.blocks.len(), 2);
    assert!(decodable(&metadata.blocks[1], MAX_DICTIONARY));
    assert!(!decodable(&metadata.blocks[1], MAX_DICTIONARY + 1));
    // An empty method ID stands for any codec the worker does not enable (PPMd, AES...).
    metadata.blocks[1].coders = vec![sevenz_rust2::Coder::default()];
    let mut archive = Archive::empty(
        Rc::new(storage::MemoryStorage::default()),
        Limits::default(),
        source.failures(),
    );
    archive.index_sevenzip(source, metadata).unwrap();
    let nodes = archive.list(1, 0).unwrap();
    assert_eq!(
        nodes.iter().map(|node| node.readable).collect::<Vec<_>>(),
        [true, false]
    );
    assert_eq!(archive.read(nodes[0].id, 0, 5).unwrap().0, b"first");
    assert_eq!(
        archive.read(nodes[1].id, 0, 5).unwrap_err().code,
        ErrorCode::Unsupported
    );
}

#[test]
fn dictionary_budget_uses_public_coder_properties() {
    // 7-Zip's normal level: LZMA2 property 24 is a 16 MiB dictionary.
    assert_eq!(
        coder_dictionary(EncoderMethod::ID_LZMA2, &[24], 0),
        16 << 20
    );
    assert_eq!(
        coder_dictionary(EncoderMethod::ID_LZMA2, &[25], 0),
        24 << 20
    );
    assert_eq!(
        coder_dictionary(EncoderMethod::ID_LZMA2, &[40], 0),
        u64::from(u32::MAX)
    );
    assert!(coder_dictionary(EncoderMethod::ID_LZMA2, &[34], 0) > MAX_DICTIONARY);
    // Malformed properties are rejected by the decoder itself when read.
    assert_eq!(coder_dictionary(EncoderMethod::ID_LZMA2, &[41], 0), 0);
    assert_eq!(coder_dictionary(EncoderMethod::ID_LZMA2, &[], 0), 0);
    // LZMA keeps at most its own output as history.
    let lzma = [0x5d, 0, 0, 0, 0x40];
    assert_eq!(
        coder_dictionary(EncoderMethod::ID_LZMA, &lzma, u64::MAX),
        1 << 30
    );
    assert_eq!(coder_dictionary(EncoderMethod::ID_LZMA, &lzma, 1000), 1000);
    assert_eq!(coder_dictionary(EncoderMethod::ID_LZMA, &lzma[..4], 0), 0);
    assert_eq!(coder_dictionary(EncoderMethod::ID_COPY, &[], 1 << 40), 0);
}
