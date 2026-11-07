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
