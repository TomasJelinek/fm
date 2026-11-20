use super::*;
use crate::text::tests::appearance;
use midnight_plugin_host::{ContentSource, Error};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Debug)]
struct Memory {
    bytes: Vec<u8>,
    reads: Mutex<Vec<(u64, u32)>>,
    version: AtomicUsize,
}

impl ContentSource for Memory {
    fn length(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn display_name(&self) -> String {
        "memory.bin".into()
    }
    fn version(&self) -> Vec<u8> {
        self.version.load(Ordering::Acquire).to_le_bytes().to_vec()
    }
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        self.reads.lock().unwrap().push((offset, length));
        let start = (offset as usize).min(self.bytes.len());
        let end = (start + length as usize).min(self.bytes.len());
        Ok(self.bytes[start..end].to_vec())
    }
}

fn memory(bytes: Vec<u8>) -> (Arc<Memory>, CapturedFile) {
    let source = Arc::new(Memory {
        bytes,
        reads: Mutex::default(),
        version: AtomicUsize::new(1),
    });
    let file = CapturedFile::from_source(source.clone()).unwrap();
    (source, file)
}

fn pair(left: Vec<u8>, right: Vec<u8>) -> [CapturedFile; 2] {
    [memory(left).1, memory(right).1]
}

fn random(length: usize, mut seed: u64) -> Vec<u8> {
    (0..length)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed as u8
        })
        .collect()
}

fn finish(bytes: &mut BinaryComparison, sources: &[CapturedFile; 2]) -> IdentityVerdict {
    let cancel = AtomicBool::new(false);
    for _ in 0..100_000 {
        match bytes.scan.step(sources, &cancel, || true).unwrap() {
            IdentityTurn::Complete(verdict) => return verdict,
            IdentityTurn::Advanced => {
                bytes.resolve_pending();
            }
            IdentityTurn::Blocked => std::thread::sleep(Duration::from_millis(1)),
        }
    }
    panic!("the byte comparison did not finish");
}

fn comparison(sources: &[CapturedFile; 2]) -> BinaryComparison {
    BinaryComparison::new(sources, [false; 2], None)
}

fn geometry(width: u32) -> ViewerGeometry {
    ViewerGeometry {
        width,
        height: 320,
        scale: 1.0,
    }
}

fn paint(bytes: &mut BinaryComparison, sources: &[CapturedFile; 2], width: u32) -> Frame {
    let appearance = appearance(12.0);
    bytes
        .paint(
            geometry(width),
            &appearance,
            Palette::from(&appearance),
            sources,
            &AtomicBool::new(false),
        )
        .unwrap()
}

#[test]
fn counts_differing_bytes_regions_and_the_first_difference() {
    let left = random(8192, 7);
    let mut right = left.clone();
    for offset in [100, 4000, 8000] {
        right[offset] ^= 0xff;
    }
    let sources = pair(left, right);
    let mut bytes = comparison(&sources);
    assert_eq!(finish(&mut bytes, &sources), IdentityVerdict::Different);
    let scan = &bytes.scan;
    assert_eq!(
        (scan.differing, scan.regions, scan.first),
        (3, 3, Some(100))
    );
    assert_eq!(scan.stops.starts, [100, 4000, 8000]);
    let status = bytes.status(offset_digits(scan.total()));
    assert!(
        status.starts_with("3 differing bytes in 3 regions · first difference at 0x0064 (100)"),
        "{status}"
    );
    assert!(status.contains("8,192 bytes each"), "{status}");
}

#[test]
fn identical_and_empty_files_are_identical_after_validating_both() {
    for content in [Vec::new(), random(300_000, 3)] {
        let sources = pair(content.clone(), content);
        let mut bytes = comparison(&sources);
        assert_eq!(
            finish(&mut bytes, &sources),
            IdentityVerdict::IdenticalFiles
        );
        assert_eq!((bytes.scan.differing, bytes.scan.regions), (0, 0));
        assert!(bytes.status(4).starts_with("Bytes identical"));
    }
}

#[test]
fn runs_across_scan_blocks_and_a_longer_file_s_tail_are_single_regions() {
    let block = SCAN_BYTES as usize;
    let left = vec![0u8; 3 * block];
    let mut right = left.clone();
    // One run across the first block boundary, one pair of separate bytes.
    for byte in &mut right[block - 5..block + 5] {
        *byte = 1;
    }
    right[2 * block] = 1;
    right[2 * block + 2] = 1;
    // The last byte differs and File B continues: the run goes on into the tail.
    *right.last_mut().unwrap() = 1;
    right.extend([9; 100]);
    let sources = pair(left, right);
    let mut bytes = comparison(&sources);
    assert_eq!(finish(&mut bytes, &sources), IdentityVerdict::Different);
    let scan = &bytes.scan;
    assert_eq!(scan.regions, 4);
    assert_eq!(scan.differing, 10 + 2 + 1 + 100);
    assert_eq!(
        scan.stops.starts,
        [
            block as u64 - 5,
            2 * block as u64,
            2 * block as u64 + 2,
            3 * block as u64 - 1
        ]
    );
    // A shorter File B: its missing bytes are one region after an equal byte.
    let sources = pair(vec![5; 10], vec![5; 4]);
    let mut bytes = comparison(&sources);
    finish(&mut bytes, &sources);
    assert_eq!(
        (bytes.scan.differing, bytes.scan.regions, bytes.scan.first),
        (6, 1, Some(4))
    );
    assert!(bytes.status(4).contains("File A 10 bytes, File B 4 bytes"));
}

#[test]
fn each_turn_reads_at_most_one_block_of_each_file_and_waits_for_io_admission() {
    let (left, left_file) = memory(random(3 * SCAN_BYTES as usize, 1));
    let (_, right_file) = memory(random(3 * SCAN_BYTES as usize, 2));
    let sources = [left_file, right_file];
    let mut bytes = comparison(&sources);
    let cancel = AtomicBool::new(false);
    let gate = midnight_work_budget::shared_io();
    {
        // Another exclusive reader holds the process-wide gate.
        let held = gate.try_acquire(Admission::Exclusive);
        if held.is_some() {
            assert_eq!(
                bytes.scan.step(&sources, &cancel, || true).unwrap(),
                IdentityTurn::Blocked
            );
            assert!(left.reads.lock().unwrap().is_empty());
        }
    }
    let advanced = loop {
        match bytes.scan.step(&sources, &cancel, || true).unwrap() {
            IdentityTurn::Blocked => std::thread::sleep(Duration::from_millis(1)),
            turn => break turn,
        }
    };
    assert_eq!(advanced, IdentityTurn::Advanced);
    assert_eq!(*left.reads.lock().unwrap(), [(0, SCAN_BYTES)]);
    assert!(bytes.status(6).starts_with("Comparing bytes… 33%"));
}

#[test]
fn cancellation_replacement_and_a_changed_file_stop_the_scan() {
    let (source, left) = memory(random(4096, 5));
    let (_, right) = memory(random(4096, 6));
    let sources = [left, right];
    let mut bytes = comparison(&sources);
    assert!(
        bytes
            .scan
            .step(&sources, &AtomicBool::new(true), || true)
            .is_err()
    );
    assert!(
        bytes
            .scan
            .step(&sources, &AtomicBool::new(false), || false)
            .is_err()
    );
    source.version.fetch_add(1, Ordering::AcqRel);
    let cancel = AtomicBool::new(false);
    let failed = loop {
        match bytes.scan.step(&sources, &cancel, || true) {
            Ok(IdentityTurn::Blocked) => std::thread::sleep(Duration::from_millis(1)),
            result => break result,
        }
    };
    assert!(failed.is_err(), "a changed file never yields a verdict");
    assert!(!bytes.scan.complete());
}

#[test]
fn the_navigation_index_stays_bounded_and_keeps_its_first_region() {
    let mut stops = Stops::default();
    for start in 0..(3 * MAX_STOPS as u64) {
        stops.push(start * 2);
    }
    assert!(stops.starts.len() <= MAX_STOPS);
    assert!(stops.spacing > 1);
    assert_eq!(stops.starts[0], 0);
    assert!(
        stops
            .starts
            .windows(2)
            .all(|pair| pair[1] - pair[0] >= stops.spacing)
    );
}

#[test]
fn helpers_find_the_same_differences_as_a_byte_loop() {
    for seed in 1..200u64 {
        let length = (seed * 37 % 300) as usize;
        let left = random(length, seed);
        let mut right = left.clone();
        let mut flip = random(length, seed + 1000);
        for (index, byte) in right.iter_mut().enumerate() {
            // Runs of differing bytes of varying length.
            if !flip[index].is_multiple_of(seed as u8 % 7 + 2) {
                *byte ^= 0x5a;
            }
        }
        flip.clear();
        let naive_first = left.iter().zip(&right).position(|(a, b)| a != b);
        assert_eq!(first_difference(&left, &right), naive_first, "seed {seed}");
        let naive_run = left.iter().zip(&right).take_while(|(a, b)| a != b).count();
        assert_eq!(differing_run(&left, &right), naive_run, "seed {seed}");
    }
}

#[test]
fn rows_show_offsets_hex_and_characters_and_mark_differing_cells() {
    let mut line = String::new();
    let mut highlights = Vec::new();
    let bytes = b"Hi\x00\x7fXY";
    let differs = [false, true, true, false, true, false, false, false];
    format_row(0x40, 4, bytes, &differs, &mut line, &mut highlights);
    // Two absent cells keep the character column aligned with full rows.
    assert_eq!(
        line,
        format!("0040  48 69 00 7F 58 59 {}│ Hi··XY", " ".repeat(6))
    );
    let marked: Vec<&str> = highlights
        .iter()
        .map(|range| &line[range.clone()])
        .collect();
    assert_eq!(marked, ["69 00", "58", "i·", "X"]);
}

#[test]
fn painting_shows_rows_of_both_files_and_marks_only_differing_bytes() {
    let left = b"The same text. One byte differs here: A, and here too: x.".to_vec();
    let mut right = left.clone();
    right[38] = b'B';
    right.truncate(50);
    let sources = pair(left, right);
    let mut bytes = comparison(&sources);
    finish(&mut bytes, &sources);
    let frame = paint(&mut bytes, &sources, 1600);
    assert!(frame.bytes);
    assert_eq!(bytes.row_bytes, 16, "a wide window shows 16 bytes per row");
    let rows: Vec<&str> = frame.accessible_text.lines().collect();
    assert_eq!(rows.len(), 4);
    assert!(rows[0].starts_with("0000  54 68 65 20"), "{}", rows[0]);
    assert!(rows[0].contains("│ The same text. O\t0000  54 68"));
    assert!(rows[2].contains("41 2C") && rows[2].split('\t').nth(1).unwrap().contains("42 2C"));
    // File B ends two bytes into the last row.
    assert!(
        rows[3].ends_with(&format!("\t0030  65 20 {}│ e ", " ".repeat(42))),
        "{}",
        rows[3]
    );
    let appearance = appearance(12.0);
    let marked = tinted(Palette::from(&appearance).background, 0, 3, 5);
    assert!(
        frame.pixels.as_chunks::<4>().0.contains(&marked),
        "differing bytes are marked"
    );
    // A narrow window shows fewer bytes per row and keeps the first visible offset.
    bytes.scroll(2);
    paint(&mut bytes, &sources, 1600);
    assert_eq!(bytes.top, 32);
    let frame = paint(&mut bytes, &sources, 500);
    assert!(bytes.row_bytes < 16);
    assert_eq!(bytes.top, 32);
    assert!(frame.accessible_text.starts_with("0020  "));
}

#[test]
fn difference_keys_visit_each_region_row_and_wait_for_regions_not_scanned_yet() {
    let length = 2 * SCAN_BYTES as usize + 4096;
    let left = vec![0u8; length];
    let mut right = left.clone();
    for offset in [64, 70, 1600, SCAN_BYTES as usize + 32, length - 1] {
        right[offset] = 1;
    }
    let sources = pair(left, right);
    let mut bytes = comparison(&sources);
    paint(&mut bytes, &sources, 1600);
    let cancel = AtomicBool::new(false);
    let step = |bytes: &mut BinaryComparison| loop {
        match bytes.scan.step(&sources, &cancel, || true).unwrap() {
            IdentityTurn::Blocked => std::thread::sleep(Duration::from_millis(1)),
            turn => break turn,
        }
    };
    // The first turn scans the first block only.
    assert_eq!(step(&mut bytes), IdentityTurn::Advanced);
    bytes.difference(false);
    assert_eq!(bytes.top, 64, "regions 64 and 70 share the row at 0x40");
    bytes.difference(false);
    assert_eq!(bytes.top, 1600);
    bytes.difference(false);
    assert_eq!(bytes.top, 1600, "the next region is not scanned yet");
    assert!(bytes.pending.is_some());
    assert!(!bytes.resolve_pending());
    assert_eq!(step(&mut bytes), IdentityTurn::Advanced);
    assert!(bytes.resolve_pending());
    assert_eq!(bytes.top, SCAN_BYTES as u64 + 32);
    bytes.difference_edge(true);
    assert!(
        bytes.pending.is_some(),
        "the last region needs the whole scan"
    );
    assert!(matches!(step(&mut bytes), IdentityTurn::Complete(_)));
    assert!(bytes.resolve_pending());
    assert_eq!(bytes.top, (length as u64 - 1) / 16 * 16);
    bytes.difference(false);
    assert_eq!(
        bytes.top,
        (length as u64 - 1) / 16 * 16,
        "Next stops at the last"
    );
    bytes.difference(true);
    assert_eq!(bytes.top, SCAN_BYTES as u64 + 32);
    bytes.difference_edge(false);
    assert_eq!(bytes.top, 64);
    bytes.difference(true);
    assert_eq!(bytes.top, 64, "Previous stops at the first");
    // Scrolling cancels a waiting jump.
    let sources = pair(vec![0; length], vec![0; length]);
    let mut identical = comparison(&sources);
    identical.difference(false);
    assert!(identical.pending.is_some());
    identical.scroll(3);
    assert!(identical.pending.is_none());
    identical.seek(1.0);
    paint(&mut identical, &sources, 1600);
    assert_eq!(identical.top, (length as u64 - 1) / 16 * 16);
}

#[test]
fn wheel_steps_smaller_than_a_row_accumulate_across_paints() {
    let sources = pair(vec![0; 4096], vec![0; 4096]);
    let mut bytes = comparison(&sources);
    paint(&mut bytes, &sources, 1600);
    let row = bytes.row_height;
    for _ in 0..4 {
        bytes.scroll_pixels(-row / 4.0);
        paint(&mut bytes, &sources, 1600);
    }
    assert_eq!(bytes.top, 16);
    bytes.scroll_pages(1);
    assert_eq!(bytes.top, 16 * (1 + bytes.page_rows));
}

/// Scan time and peak memory for two native files of `MIDNIGHT_SCAN_MIB` MiB (default
/// 256) that differ in one byte of every 4 KiB, captured like the default service does.
#[test]
#[ignore = "timing harness; run with --ignored --nocapture"]
fn byte_scan_timing() {
    use std::io::Write as _;
    let mebibytes: usize = std::env::var("MIDNIGHT_SCAN_MIB")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(256);
    let directory = tempfile::tempdir().unwrap();
    let paths = ["a.bin", "b.bin"].map(|name| directory.path().join(name));
    let chunk = random(1 << 20, 11);
    let mut changed = chunk.clone();
    for byte in changed.iter_mut().step_by(4096) {
        *byte ^= 0x55;
    }
    for (path, content) in paths.iter().zip([&chunk, &changed]) {
        let mut file = std::fs::File::create(path).unwrap();
        for _ in 0..mebibytes {
            file.write_all(content).unwrap();
        }
    }
    let sources = paths
        .each_ref()
        .map(|path| CapturedFile::capture(path).unwrap());
    let peak = || {
        std::fs::read_to_string("/proc/self/status")
            .unwrap_or_default()
            .lines()
            .find(|line| line.starts_with("VmHWM"))
            .unwrap_or_default()
            .to_owned()
    };
    let before = peak();
    let mut bytes = comparison(&sources);
    let started = Instant::now();
    assert_eq!(finish(&mut bytes, &sources), IdentityVerdict::Different);
    let elapsed = started.elapsed();
    println!(
        "byte scan: {mebibytes} MiB per file in {:.0} ms ({:.0} MiB/s), {} regions, \
         {} stops; {before} -> {}",
        elapsed.as_secs_f64() * 1e3,
        mebibytes as f64 / elapsed.as_secs_f64(),
        bytes.scan.regions,
        bytes.scan.stops.starts.len(),
        peak()
    );
}
