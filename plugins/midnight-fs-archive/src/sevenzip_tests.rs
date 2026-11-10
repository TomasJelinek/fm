//! Check the pinned decoder's block-only integrity path, independently of member CRCs.
use crate::*;
use sevenz_rust2::{
    ArchiveEntry, ArchiveWriter, EncoderConfiguration, EncoderMethod, SourceReader,
};
use std::io::Cursor;

fn solid() -> Archive {
    let mut writer = ArchiveWriter::new(Cursor::new(Vec::new())).unwrap();
    writer.set_content_methods(vec![EncoderConfiguration::new(EncoderMethod::COPY)]);
    writer
        .push_archive_entries(
            vec![ArchiveEntry::new_file("a"), ArchiveEntry::new_file("b")],
            vec![
                SourceReader::new(Cursor::new(b"first")),
                SourceReader::new(Cursor::new(b"other")),
            ],
        )
        .unwrap();
    Archive::open(
        Format::SevenZip,
        writer.finish().unwrap().into_inner(),
        Limits::default(),
    )
    .unwrap()
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
    content.source = std::io::BufReader::new(Box::new(Cursor::new(Vec::<u8>::new())));
    assert_eq!(archive.read(2, 0, 1).unwrap_err().code, ErrorCode::Corrupt);
    assert!(archive.contents.is_empty());
}

#[test]
fn only_selected_member_is_retained_after_solid_block_verification() {
    let mut archive = solid();
    assert_eq!(archive.read(2, 0, 1).unwrap().0, b"f");
    // Only the selected member is retained; a sibling requires another block pass.
    archive.sevenzip_content.as_mut().unwrap().source =
        std::io::BufReader::new(Box::new(Cursor::new(Vec::<u8>::new())));
    assert_eq!(archive.read(2, 0, 5).unwrap().0, b"first");
    assert!(archive.read(3, 0, 5).is_err());
}
