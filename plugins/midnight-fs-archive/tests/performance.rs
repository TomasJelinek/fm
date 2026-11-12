//! Manual development measurement; no wall-clock thresholds in ordinary CI.
use midnight_filesystem_api::Format;
use midnight_fs_archive::{Archive, Limits};
use std::{
    io::{Cursor, Write},
    time::Instant,
};

#[test]
#[ignore = "manual archive opening and pagination measurement"]
fn measure_open_and_list() {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let payload: Vec<_> = (0..8192).map(|i| ((i * 17 + i / 19) % 251) as u8).collect();
    for i in 0..8000 {
        writer.start_file(format!("file-{i:05}"), options).unwrap();
        writer.write_all(&payload).unwrap();
    }
    let bytes = writer.finish().unwrap().into_inner();
    let start = Instant::now();
    let mut archive = Archive::open(Format::Zip, bytes, Limits::default()).unwrap();
    let opened = start.elapsed();
    let start = Instant::now();
    let mut after = 0;
    let mut count = 0;
    loop {
        let batch = archive.list(1, after).unwrap();
        if batch.is_empty() {
            break;
        }
        after = batch.last().unwrap().id;
        count += batch.len();
    }
    let listed = start.elapsed();
    let start = Instant::now();
    assert_eq!(archive.read(2, 0, 8192).unwrap().0, payload);
    eprintln!(
        "8,000 ZIP entries / 62.5 MiB decoded: open={opened:?}, list={listed:?}, first_read={:?}",
        start.elapsed()
    );
    assert_eq!(count, 8000);
}

/// Same deterministic 16 MiB payload, independently compressed blocks or one solid block.
/// Fixture generation and source cloning are excluded from the reported timings.
#[test]
#[ignore = "manual TAR and 7z opening measurement; run --release --ignored --nocapture"]
fn measure_tar_and_sevenzip_open() {
    let mut random = 0x7823_a561u32;
    let payloads: Vec<Vec<u8>> = (0..64)
        .map(|_| {
            (0..256 * 1024)
                .map(|_| {
                    random ^= random << 13;
                    random ^= random >> 17;
                    random ^= random << 5;
                    (random & 15) as u8
                })
                .collect()
        })
        .collect();
    let mut tar = tar::Builder::new(Vec::new());
    for (i, bytes) in payloads.iter().enumerate() {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o600);
        header.set_cksum();
        tar.append_data(&mut header, format!("file-{i:03}"), bytes.as_slice())
            .unwrap();
    }
    measure("TAR", Format::Tar, tar.into_inner().unwrap(), &payloads);
    for solid in [false, true] {
        let mut writer = sevenz_rust2::ArchiveWriter::new(Cursor::new(Vec::new())).unwrap();
        if solid {
            writer
                .push_archive_entries(
                    (0..64)
                        .map(|i| sevenz_rust2::ArchiveEntry::new_file(&format!("file-{i:03}")))
                        .collect(),
                    payloads
                        .iter()
                        .map(|bytes| sevenz_rust2::SourceReader::new(Cursor::new(bytes.as_slice())))
                        .collect(),
                )
                .unwrap();
        } else {
            for (i, bytes) in payloads.iter().enumerate() {
                writer
                    .push_archive_entry(
                        sevenz_rust2::ArchiveEntry::new_file(&format!("file-{i:03}")),
                        Some(Cursor::new(bytes.as_slice())),
                    )
                    .unwrap();
            }
        }
        measure(
            if solid { "7z solid" } else { "7z independent" },
            Format::SevenZip,
            writer.finish().unwrap().into_inner(),
            &payloads,
        );
    }
}

fn measure(name: &str, format: Format, bytes: Vec<u8>, payloads: &[Vec<u8>]) {
    if let Some(directory) = std::env::var_os("MIDNIGHT_ARCHIVE_PERF_DIR") {
        let directory = std::path::PathBuf::from(directory);
        assert!(
            directory
                .components()
                .any(|part| part.as_os_str() == "_build")
        );
        std::fs::create_dir_all(&directory).unwrap();
        let filename = match name {
            "TAR" => "payload-16m.tar",
            "7z solid" => "payload-16m-solid.7z",
            _ => "payload-16m-independent.7z",
        };
        std::fs::write(directory.join(filename), &bytes).unwrap();
    }
    for sample in 0..5 {
        let source = bytes.clone();
        let start = Instant::now();
        let mut archive = Archive::open(format, source, Limits::default()).unwrap();
        let opened = start.elapsed();
        let start = Instant::now();
        let nodes = archive.list(1, 0).unwrap();
        let listed = start.elapsed();
        assert_eq!(nodes.len(), 64);
        let start = Instant::now();
        assert_eq!(
            archive.read(nodes[0].id, 0, 256 * 1024).unwrap().0,
            payloads[0]
        );
        let first_read = start.elapsed();
        let start = Instant::now();
        assert_eq!(
            archive.read(nodes[63].id, 0, 256 * 1024).unwrap().0,
            payloads[63]
        );
        eprintln!(
            "{name} sample={sample} source={} open={opened:?} list={listed:?} first_read={first_read:?} last_read={:?}",
            bytes.len(),
            start.elapsed()
        );
    }
}
