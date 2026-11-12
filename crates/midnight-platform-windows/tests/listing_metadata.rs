#![cfg(windows)]
use midnight_core::{Cancellation, DirectoryService};
use midnight_platform_windows::WindowsFileSystem;
use std::{fs, time::Instant};

#[test]
fn enumeration_includes_native_metadata_without_opening_files() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("文档.bin");
    fs::write(&path, b"contents").unwrap();
    let filesystem = WindowsFileSystem::new(None);
    filesystem
        .read_directory(directory.path(), &Cancellation::default(), &mut |entry| {
            assert_eq!(entry.metadata, filesystem.entry_metadata(&path).unwrap());
            true
        })
        .unwrap();
}

#[test]
#[ignore = "local listing performance measurement"]
fn measure_listing_with_metadata() {
    let directory = tempfile::tempdir().unwrap();
    for index in 0..4000 {
        fs::write(
            directory.path().join(format!("file-{index:05}.txt")),
            b"data",
        )
        .unwrap();
    }
    let filesystem = WindowsFileSystem::new(None);
    let mut times = Vec::new();
    for _ in 0..7 {
        let started = Instant::now();
        let mut entries = Vec::new();
        filesystem
            .read_directory(directory.path(), &Cancellation::default(), &mut |entry| {
                entries.push(entry);
                true
            })
            .unwrap();
        for entry in &mut entries {
            if entry.metadata.native_attributes.is_none() {
                entry.metadata = filesystem.entry_metadata(&entry.path).unwrap();
            }
        }
        assert_eq!(entries.len(), 4000);
        assert!(entries.iter().all(|entry| entry.metadata.size == Some(4)));
        times.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(f64::total_cmp);
    println!(
        "4000 entries+metadata: median {:.3} ms; sorted runs {times:?}",
        times[3]
    );
}
