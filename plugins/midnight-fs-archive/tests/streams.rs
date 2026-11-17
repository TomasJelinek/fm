use midnight_filesystem_api::{ErrorCode, Format};
use midnight_fs_archive::{Archive, Limits};
use std::io::Write;

fn compressed(format: Format, bytes: &[u8]) -> Vec<u8> {
    match format {
        Format::Xz | Format::TarXz => {
            let mut writer =
                lzma_rust2::XzWriter::new(Vec::new(), lzma_rust2::XzOptions::default()).unwrap();
            writer.write_all(bytes).unwrap();
            writer.finish().unwrap()
        }
        Format::Bzip2 | Format::TarBzip2 => {
            let mut writer =
                bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::default());
            writer.write_all(bytes).unwrap();
            writer.finish().unwrap()
        }
        _ => ruzstd::encoding::compress_to_vec(
            bytes,
            ruzstd::encoding::CompressionLevel::Uncompressed,
        ),
    }
}
#[test]
fn standalone_streams_multimember_and_truncation() {
    for format in [Format::Xz, Format::Bzip2, Format::Zstd] {
        let mut bytes = compressed(format, b"hello");
        bytes.extend(compressed(format, b" world"));
        let mut archive = Archive::open(format, bytes.clone(), Limits::default()).unwrap();
        let node = archive.list(1, 0).unwrap().remove(0);
        assert_eq!(archive.read(node.id, 0, 100).unwrap().0, b"hello world");
        bytes.pop();
        assert!(
            Archive::open(format, bytes, Limits::default()).is_err(),
            "{format:?}"
        );
    }
}
#[test]
fn compressed_tar_preserves_unicode_and_payload() {
    let mut tar = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_size(5);
    header.set_mode(0o644);
    header.set_cksum();
    tar.append_data(&mut header, "日本語.txt", &b"hello"[..])
        .unwrap();
    let bytes = tar.into_inner().unwrap();
    for format in [Format::TarXz, Format::TarBzip2, Format::TarZstd] {
        let mut archive =
            Archive::open(format, compressed(format, &bytes), Limits::default()).unwrap();
        let node = archive.list(1, 0).unwrap().remove(0);
        assert_eq!(node.display, "日本語.txt");
        assert_eq!(archive.read(node.id, 0, 100).unwrap().0, b"hello");
    }
}
#[test]
fn streams_obey_actual_output_budgets() {
    for format in [Format::Xz, Format::Bzip2, Format::Zstd] {
        let limits = Limits {
            max_total: 8,
            ..Limits::default()
        };
        assert_eq!(
            Archive::open(format, compressed(format, b"ninebytes"), limits)
                .err()
                .unwrap()
                .code,
            ErrorCode::Limit
        );
    }
}
#[test]
fn zstandard_skippable_frames_are_skipped_but_must_be_complete() {
    let skippable = |declared: u32, present: usize| {
        let mut frame = 0x184d_2a50u32.to_le_bytes().to_vec();
        frame.extend(declared.to_le_bytes());
        frame.extend(vec![0xee; present]);
        frame
    };
    let mut bytes = skippable(100, 100);
    bytes.extend(compressed(Format::Zstd, b"hello"));
    bytes.extend(skippable(3, 3));
    let mut archive = Archive::open(Format::Zstd, bytes, Limits::default()).unwrap();
    let node = archive.list(1, 0).unwrap().remove(0);
    assert_eq!(archive.read(node.id, 0, 100).unwrap().0, b"hello");
    let mut truncated = compressed(Format::Zstd, b"hello");
    truncated.extend(skippable(100, 10));
    assert_eq!(
        Archive::open(Format::Zstd, truncated, Limits::default())
            .err()
            .unwrap()
            .code,
        ErrorCode::Corrupt
    );
}

#[test]
fn zstandard_checksum_is_checked_before_output_publication() {
    let mut bytes = compressed(Format::Zstd, b"hello");
    // Set the frame's checksum flag and append a deliberately invalid checksum.
    bytes[4] |= 4;
    bytes.extend([0, 0, 0, 0]);
    assert!(Archive::open(Format::Zstd, bytes, Limits::default()).is_err());
}
