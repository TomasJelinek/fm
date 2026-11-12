//! Owned cross-process fixtures. Output is explicit and stays under _build.
use std::{
    fs,
    io::{Cursor, Write},
    path::PathBuf,
};
const PAYLOAD: &[u8] = b"shared verified content";

#[test]
#[ignore = "exports owned fixtures to MIDNIGHT_ARCHIVE_FIXTURES under _build"]
fn export_worker_fixtures() {
    let root =
        PathBuf::from(std::env::var_os("MIDNIGHT_ARCHIVE_FIXTURES").expect("fixture output"));
    assert!(root.components().any(|part| part.as_os_str() == "_build"));
    fs::create_dir_all(&root).unwrap();
    let mut tar = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_size(PAYLOAD.len() as u64);
    header.set_mode(0o600);
    header.set_cksum();
    tar.append_data(&mut header, "member.txt", PAYLOAD).unwrap();
    let tar = tar.into_inner().unwrap();
    fs::write(root.join("owned.tar"), &tar).unwrap();
    for (source, prefix) in [(PAYLOAD, "owned"), (tar.as_slice(), "owned.tar")] {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(source).unwrap();
        fs::write(root.join(format!("{prefix}.gz")), gz.finish().unwrap()).unwrap();
        let mut xz =
            lzma_rust2::XzWriter::new(Vec::new(), lzma_rust2::XzOptions::default()).unwrap();
        xz.write_all(source).unwrap();
        fs::write(root.join(format!("{prefix}.xz")), xz.finish().unwrap()).unwrap();
        let mut bz = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::default());
        bz.write_all(source).unwrap();
        fs::write(root.join(format!("{prefix}.bz2")), bz.finish().unwrap()).unwrap();
        fs::write(
            root.join(format!("{prefix}.zst")),
            ruzstd::encoding::compress_to_vec(
                source,
                ruzstd::encoding::CompressionLevel::Uncompressed,
            ),
        )
        .unwrap();
    }
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file(
        "member.txt",
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
    )
    .unwrap();
    zip.write_all(PAYLOAD).unwrap();
    let zip = zip.finish().unwrap().into_inner();
    fs::write(root.join("owned.zip"), &zip).unwrap();
    let mut corrupt = zip;
    let offset = corrupt
        .windows(PAYLOAD.len())
        .position(|bytes| bytes == PAYLOAD)
        .unwrap();
    corrupt[offset] ^= 1;
    fs::write(root.join("corrupt.zip"), corrupt).unwrap();
    let mut seven = sevenz_rust2::ArchiveWriter::new(Cursor::new(Vec::new())).unwrap();
    seven.set_content_methods(vec![sevenz_rust2::EncoderConfiguration::new(
        sevenz_rust2::EncoderMethod::COPY,
    )]);
    seven
        .push_archive_entries(
            vec![
                sevenz_rust2::ArchiveEntry::new_file("member.txt"),
                sevenz_rust2::ArchiveEntry::new_file("../discard"),
            ],
            vec![
                sevenz_rust2::SourceReader::new(Cursor::new(PAYLOAD.to_vec())),
                sevenz_rust2::SourceReader::new(Cursor::new(vec![0x5a; 1024 * 1024])),
            ],
        )
        .unwrap();
    let seven = seven.finish().unwrap().into_inner();
    fs::write(root.join("owned.7z"), &seven).unwrap();
    let mut corrupt = seven;
    corrupt[32 + PAYLOAD.len() + 512] ^= 1;
    fs::write(root.join("corrupt.7z"), corrupt).unwrap();
}

#[test]
#[ignore = "exports cooperative cancellation fixtures under _build"]
fn export_cooperative_fixtures() {
    let root =
        PathBuf::from(std::env::var_os("MIDNIGHT_ARCHIVE_FIXTURES").expect("fixture output"));
    assert!(root.components().any(|part| part.as_os_str() == "_build"));
    fs::create_dir_all(&root).unwrap();
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("cached.bin", options).unwrap();
    zip.write_all(b"keep").unwrap();
    zip.start_file("large.bin", options).unwrap();
    zip.write_all(&vec![42; 2 * 1024 * 1024]).unwrap();
    fs::write(root.join("cancel.zip"), zip.finish().unwrap().into_inner()).unwrap();
    let mut seven = sevenz_rust2::ArchiveWriter::new(Cursor::new(Vec::new())).unwrap();
    seven.set_content_methods(vec![sevenz_rust2::EncoderConfiguration::new(
        sevenz_rust2::EncoderMethod::COPY,
    )]);
    seven
        .push_archive_entries(
            vec![
                sevenz_rust2::ArchiveEntry::new_file("cached.bin"),
                sevenz_rust2::ArchiveEntry::new_file("../discard"),
                sevenz_rust2::ArchiveEntry::new_file("large.bin"),
            ],
            vec![
                sevenz_rust2::SourceReader::new(Cursor::new(b"keep".to_vec())),
                sevenz_rust2::SourceReader::new(Cursor::new(vec![3; 2 * 1024 * 1024])),
                sevenz_rust2::SourceReader::new(Cursor::new(vec![42; 4])),
            ],
        )
        .unwrap();
    fs::write(root.join("cancel.7z"), seven.finish().unwrap().into_inner()).unwrap();
}
