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

/// Index memory per member, which the host derives the worker's memory policy from.
/// Each case runs in a fresh child process of this test binary, which reports how far
/// its peak resident set grew while it opened and then completely listed a generated
/// archive of empty members, 1,000 to a folder, and what the open index retained.
/// Archives are written beneath `MIDNIGHT_ARCHIVE_PERF_DIR` (under `_build/`) and
/// removed afterwards unless `MIDNIGHT_ARCHIVE_MEMORY_KEEP` is set, which retains them
/// for host and sandbox checks; `MIDNIGHT_ARCHIVE_MEMORY_ENTRIES` sets the member count
/// (default 1,000,000) and `MIDNIGHT_ARCHIVE_MEMORY_FORMATS` the formats (default
/// `tar,tar.gz,zip,7z,7z-raw`; `7z-raw` has an uncompressed header). Linux only: it
/// reads /proc/self/status.
#[cfg(target_os = "linux")]
#[test]
#[ignore = "manual index memory measurement; run --release --ignored --nocapture"]
fn measure_index_memory() {
    match std::env::var(memory::CASE) {
        Ok(case) => memory::child(&case),
        Err(_) => memory::parent(),
    }
}

#[cfg(target_os = "linux")]
#[path = "support/raw_sevenzip.rs"]
mod raw_sevenzip;

#[cfg(target_os = "linux")]
mod memory {
    use midnight_filesystem_api::{Format, Kind, StagePurpose};
    use midnight_fs_archive::{Archive, Limits, storage::Storage};
    use std::{
        cell::Cell,
        fs::File,
        io::{self, BufWriter, Write},
        path::{Path, PathBuf},
        process::Command,
        rc::Rc,
        time::Instant,
    };

    pub(super) const CASE: &str = "MIDNIGHT_ARCHIVE_MEMORY_CASE";

    /// Staged scratch is counted, not kept, so only the index stays in memory.
    #[derive(Default)]
    struct Discard(Cell<u64>);
    impl Storage for Discard {
        fn begin(&self, _: StagePurpose, _: Option<u64>, _: u64) -> io::Result<u64> {
            self.0.set(self.0.get() + 1);
            Ok(self.0.get())
        }
        fn append(&self, _: u64, _: u64, _: &[u8]) -> io::Result<()> {
            Ok(())
        }
        fn seal(&self, _: u64, _: u64) -> io::Result<()> {
            Ok(())
        }
        fn read(&self, _: u64, _: u64, length: u32) -> io::Result<Vec<u8>> {
            Ok(vec![0; length as usize])
        }
    }

    fn format(name: &str) -> Format {
        match name {
            "tar" => Format::Tar,
            "tar.gz" => Format::TarGzip,
            "zip" => Format::Zip,
            "7z" | "7z-raw" => Format::SevenZip,
            _ => panic!("unknown measured format {name}"),
        }
    }

    fn member(index: usize) -> String {
        format!("dir-{:04}/file-{index:07}.txt", index / 1000)
    }

    fn tar_members<W: Write>(output: W, entries: usize) -> W {
        let mut builder = tar::Builder::new(output);
        for index in 0..entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(0);
            header.set_mode(0o644);
            header.set_mtime(1_700_000_000);
            builder
                .append_data(&mut header, member(index), io::empty())
                .unwrap();
        }
        builder.into_inner().unwrap()
    }

    fn generate(name: &str, entries: usize, path: &Path) {
        let file = BufWriter::new(File::create(path).unwrap());
        match name {
            "tar" => {
                tar_members(file, entries).flush().unwrap();
            }
            "tar.gz" => {
                let gzip = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
                tar_members(gzip, entries)
                    .finish()
                    .unwrap()
                    .flush()
                    .unwrap();
            }
            "zip" => {
                let mut writer = zip::ZipWriter::new(file);
                let options = zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored);
                for index in 0..entries {
                    writer.start_file(member(index), options).unwrap();
                }
                writer.finish().unwrap().flush().unwrap();
            }
            "7z" => {
                // One byte each, a solid block per folder: an empty 7z member has no
                // stream, so the decoder would keep less metadata than for real files.
                let mut writer = sevenz_rust2::ArchiveWriter::new(file).unwrap();
                writer.set_content_methods(vec![sevenz_rust2::EncoderConfiguration::new(
                    sevenz_rust2::EncoderMethod::COPY,
                )]);
                for first in (0..entries).step_by(1000) {
                    let folder = first..entries.min(first + 1000);
                    writer
                        .push_archive_entries(
                            folder
                                .clone()
                                .map(|index| sevenz_rust2::ArchiveEntry::new_file(&member(index)))
                                .collect(),
                            folder
                                .map(|_| sevenz_rust2::SourceReader::new(&b"x"[..]))
                                .collect(),
                        )
                        .unwrap();
                }
                writer.finish().unwrap().flush().unwrap();
            }
            "7z-raw" => {
                let names: Vec<String> = (0..entries).map(member).collect();
                let mut file = file;
                file.write_all(&super::raw_sevenzip::archive(&names))
                    .unwrap();
                file.flush().unwrap();
            }
            _ => panic!("unknown measured format {name}"),
        }
    }

    /// Peak (`VmHWM`) and current (`VmRSS`) resident set in KiB. The address space
    /// (`VmPeak`) says nothing here: the test thread's allocator arena reserved it.
    fn status() -> [u64; 2] {
        let text = std::fs::read_to_string("/proc/self/status").unwrap();
        ["VmHWM:", "VmRSS:"].map(|field| {
            text.lines()
                .find_map(|line| line.strip_prefix(field))
                .and_then(|value| value.trim().trim_end_matches("kB").trim().parse().ok())
                .unwrap()
        })
    }

    /// Files and folders of the whole tree, listed page by page like the host does.
    fn list_all(archive: &Archive) -> (usize, usize) {
        let (mut files, mut folders) = (0, 0);
        let mut pending = vec![1];
        while let Some(directory) = pending.pop() {
            let mut after = 0;
            loop {
                let (page, more) = archive.page(directory, after).unwrap();
                for node in &page {
                    if node.kind == Kind::Directory {
                        folders += 1;
                        pending.push(node.id);
                    } else {
                        files += 1;
                    }
                }
                after = page.last().map_or(after, |node| node.id);
                if !more {
                    break;
                }
            }
        }
        (files, folders)
    }

    pub(super) fn child(case: &str) {
        let mut parts = case.splitn(3, '|');
        let (name, entries, path) = (
            parts.next().unwrap(),
            parts.next().unwrap().parse::<usize>().unwrap(),
            PathBuf::from(parts.next().unwrap()),
        );
        let before = status();
        let file = File::open(&path).unwrap();
        let length = file.metadata().unwrap().len();
        let start = Instant::now();
        let archive = Archive::open_stream(
            format(name),
            Box::new(file),
            length,
            Rc::new(Discard::default()),
            Limits::default(),
        )
        .unwrap();
        let opened = start.elapsed();
        let indexed = status();
        let start = Instant::now();
        let (files, folders) = list_all(&archive);
        let listed = start.elapsed();
        let after = status();
        assert_eq!(files, entries);
        assert_eq!(folders, entries.div_ceil(1000));
        let [peak, resident] = [
            after[0].saturating_sub(before[0]),
            indexed[1].saturating_sub(before[1]),
        ];
        let per_entry = |kib: u64| kib * 1024 / entries.max(1) as u64;
        println!(
            "{name}: {entries} members, source {:.1} MiB, open {opened:.2?}, list {listed:.2?}; \
             peak RSS +{:.1} MiB ({} B/member), retained RSS +{:.1} MiB ({} B/member)",
            length as f64 / 1048576.0,
            peak as f64 / 1024.0,
            per_entry(peak),
            resident as f64 / 1024.0,
            per_entry(resident),
        );
    }

    pub(super) fn parent() {
        let directory = PathBuf::from(
            std::env::var_os("MIDNIGHT_ARCHIVE_PERF_DIR").expect("MIDNIGHT_ARCHIVE_PERF_DIR"),
        );
        assert!(
            directory
                .components()
                .any(|part| part.as_os_str() == "_build")
        );
        std::fs::create_dir_all(&directory).unwrap();
        let entries = std::env::var("MIDNIGHT_ARCHIVE_MEMORY_ENTRIES")
            .map_or(1_000_000, |value| value.parse().unwrap());
        let formats = std::env::var("MIDNIGHT_ARCHIVE_MEMORY_FORMATS")
            .unwrap_or_else(|_| "tar,tar.gz,zip,7z,7z-raw".into());
        for name in formats.split(',') {
            let path = directory.join(format!("members-{entries}.{name}"));
            let start = Instant::now();
            generate(name, entries, &path);
            eprintln!("generated {} in {:.2?}", path.display(), start.elapsed());
            let output = Command::new(std::env::current_exe().unwrap())
                .args([
                    "measure_index_memory",
                    "--exact",
                    "--ignored",
                    "--nocapture",
                ])
                .arg("--test-threads=1")
                .env(CASE, format!("{name}|{entries}|{}", path.display()))
                .output()
                .unwrap();
            if std::env::var_os("MIDNIGHT_ARCHIVE_MEMORY_KEEP").is_none() {
                std::fs::remove_file(&path).unwrap();
            }
            let report = String::from_utf8_lossy(&output.stdout);
            let prefix = format!("{name}: ");
            let line = report
                .lines()
                .find_map(|line| line.find(&prefix).map(|at| &line[at..]));
            assert!(
                output.status.success() && line.is_some(),
                "{report}\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            eprintln!("{}", line.unwrap_or_default());
        }
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
