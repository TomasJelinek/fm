use super::*;
use midnight_plugin_host::{ContentSource, Error};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Debug)]
struct Bytes {
    bytes: Vec<u8>,
    version: AtomicUsize,
    requests: Mutex<Vec<(u64, u32)>>,
    change_after_read: bool,
    short: bool,
    replace_after_read: Option<Arc<AtomicBool>>,
}

impl ContentSource for Bytes {
    fn length(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn display_name(&self) -> String {
        "image.bmp".into()
    }
    fn version(&self) -> Vec<u8> {
        self.version.load(Ordering::Acquire).to_le_bytes().to_vec()
    }
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
    fn read_at(&self, offset: u64, amount: u32) -> Result<Vec<u8>, Error> {
        self.requests.lock().unwrap().push((offset, amount));
        let start = offset as usize;
        let end = (start + amount as usize).min(self.bytes.len());
        if self.change_after_read {
            self.version.fetch_add(1, Ordering::AcqRel);
        }
        if let Some(current) = &self.replace_after_read {
            current.store(false, Ordering::Release);
        }
        let end = end.saturating_sub(usize::from(self.short)).max(start);
        Ok(self.bytes[start..end].to_vec())
    }
}

fn bytes(data: Vec<u8>) -> Arc<Bytes> {
    Arc::new(Bytes {
        bytes: data,
        version: AtomicUsize::new(1),
        requests: Mutex::new(Vec::new()),
        change_after_read: false,
        short: false,
        replace_after_read: None,
    })
}

fn captured(source: &Arc<Bytes>) -> CapturedFile {
    CapturedFile::from_source(source.clone()).unwrap()
}

fn verifier(sources: &[CapturedFile; 2], decoded_equal: Option<bool>) -> WholeInput {
    WholeInput {
        gate: Arc::new(Gate::new(4)),
        ..WholeInput::new(sources, decoded_equal, false)
    }
}

fn finish(work: &mut WholeInput, sources: &[CapturedFile; 2]) -> Result<IdentityVerdict, String> {
    let cancel = AtomicBool::new(false);
    for _ in 0..1000 {
        match work.step(sources, &cancel, || true)? {
            IdentityTurn::Complete(verdict) => return Ok(verdict),
            IdentityTurn::Advanced => {}
            IdentityTurn::Blocked => panic!("The isolated test gate has no other owner"),
        }
    }
    panic!("Whole-input verification did not finish within its bounded test input");
}

// A valid 1 by 1, 24-bit BMP. Its stored BGR pixel is blue, with one padding byte.
fn bmp() -> Vec<u8> {
    vec![
        0x42, 0x4d, 58, 0, 0, 0, 0, 0, 0, 0, 54, 0, 0, 0, 40, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1,
        0, 24, 0, 0, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 255, 0,
        0, 0,
    ]
}

#[test]
fn complete_identical_image_bytes_prove_file_identity() {
    let left = bytes(bmp());
    let right = bytes(bmp());
    let sources = [captured(&left), captured(&right)];
    assert_eq!(
        finish(&mut verifier(&sources, None), &sources).unwrap(),
        IdentityVerdict::IdenticalFiles
    );
}

#[test]
fn equal_image_pixels_with_different_encoding_metadata_are_different_files() {
    let left = bytes(bmp());
    let mut other = bmp();
    // Horizontal pixels-per-meter changes encoding metadata, leaving the pixel untouched.
    other[38] = 1;
    let right = bytes(other);
    let sources = [captured(&left), captured(&right)];
    assert_eq!(
        finish(&mut verifier(&sources, None), &sources).unwrap(),
        IdentityVerdict::Different
    );
}

#[test]
fn byte_witness_beyond_first_turn_cannot_be_missed_and_each_turn_is_bounded() {
    let left = bytes(vec![7; 196_608]);
    let mut other = vec![7; 196_608];
    other[150_000] = 8;
    let right = bytes(other);
    let sources = [captured(&left), captured(&right)];
    let mut work = verifier(&sources, None);
    assert_eq!(
        work.step(&sources, &AtomicBool::new(false), || true)
            .unwrap(),
        IdentityTurn::Advanced
    );
    assert_eq!(left.requests.lock().unwrap().len(), 1);
    assert_eq!(right.requests.lock().unwrap().len(), 1);
    assert_eq!(
        finish(&mut work, &sources).unwrap(),
        IdentityVerdict::Different
    );
    for source in [&left, &right] {
        assert!(
            source
                .requests
                .lock()
                .unwrap()
                .iter()
                .all(|(_, size)| *size <= 65_536)
        );
    }
}

#[test]
fn decoded_text_proof_distinguishes_bom_bytes_from_different_text() {
    let left = bytes(b"same\n".to_vec());
    let right = bytes(b"\xef\xbb\xbfsame\n".to_vec());
    let sources = [captured(&left), captured(&right)];
    assert_eq!(
        finish(&mut verifier(&sources, Some(true)), &sources).unwrap(),
        IdentityVerdict::IdenticalText
    );
    assert_eq!(
        finish(&mut verifier(&sources, Some(false)), &sources).unwrap(),
        IdentityVerdict::Different
    );
}

#[test]
fn cancellation_discards_identity_without_starting_a_read() {
    let left = bytes(bmp());
    let right = bytes(bmp());
    let sources = [captured(&left), captured(&right)];
    let mut work = verifier(&sources, None);
    assert!(
        work.step(&sources, &AtomicBool::new(true), || true)
            .is_err()
    );
    assert!(left.requests.lock().unwrap().is_empty());
    assert!(right.requests.lock().unwrap().is_empty());
}

#[test]
fn changed_source_and_short_reads_never_yield_a_verdict() {
    for short in [false, true] {
        let left = bytes(bmp());
        let mut right = bytes(bmp());
        let source = Arc::get_mut(&mut right).unwrap();
        source.short = short;
        source.change_after_read = !short;
        let sources = [captured(&left), captured(&right)];
        assert!(finish(&mut verifier(&sources, None), &sources).is_err());
    }
}

#[test]
fn a_different_capture_cannot_finish_an_old_pairs_proof() {
    let left = bytes(bmp());
    let right = bytes(bmp());
    let original = [captured(&left), captured(&right)];
    let replacement = [captured(&left), captured(&right)];
    let mut work = verifier(&original, None);
    assert!(
        work.step(&replacement, &AtomicBool::new(false), || true)
            .is_err()
    );
}

#[test]
fn empty_files_require_revision_validation_before_identity() {
    let left = bytes(Vec::new());
    let right = bytes(Vec::new());
    let sources = [captured(&left), captured(&right)];
    let mut work = verifier(&sources, None);
    right.version.store(2, Ordering::Release);
    assert!(
        work.step(&sources, &AtomicBool::new(false), || true)
            .is_err()
    );
}

#[test]
fn unavailable_io_capacity_yields_without_reading_or_queuing() {
    let left = bytes(bmp());
    let right = bytes(bmp());
    let sources = [captured(&left), captured(&right)];
    let mut work = verifier(&sources, None);
    let held = work.gate.try_acquire(storage_admission(false)).unwrap();
    assert_eq!(
        work.step(&sources, &AtomicBool::new(false), || true)
            .unwrap(),
        IdentityTurn::Blocked
    );
    assert!(left.requests.lock().unwrap().is_empty());
    assert!(right.requests.lock().unwrap().is_empty());
    assert_eq!(work.gate.snapshot().queued, 0);
    drop(held);
    assert_eq!(
        finish(&mut work, &sources).unwrap(),
        IdentityVerdict::IdenticalFiles
    );
}

#[test]
fn replacement_during_a_read_stops_before_reading_the_other_source() {
    let current = Arc::new(AtomicBool::new(true));
    let mut left = bytes(bmp());
    Arc::get_mut(&mut left).unwrap().replace_after_read = Some(current.clone());
    let right = bytes(bmp());
    let sources = [captured(&left), captured(&right)];
    let mut work = verifier(&sources, None);
    assert!(
        work.step(&sources, &AtomicBool::new(false), || current
            .load(Ordering::Acquire))
            .is_err()
    );
    assert!(right.requests.lock().unwrap().is_empty());
}

#[test]
fn progressing_raw_verification_uses_exactly_one_bounded_pair_of_reads_per_turn() {
    let left = bytes(vec![7; 196_608]);
    let right = bytes(vec![7; 196_608]);
    let sources = [captured(&left), captured(&right)];
    let mut work = verifier(&sources, None);
    let cancel = AtomicBool::new(false);
    assert_eq!(
        work.step(&sources, &cancel, || true).unwrap(),
        IdentityTurn::Advanced
    );
    assert_eq!(*left.requests.lock().unwrap(), [(0, 65_536)]);
    assert_eq!(*right.requests.lock().unwrap(), [(0, 65_536)]);
    assert_eq!(
        work.step(&sources, &cancel, || true).unwrap(),
        IdentityTurn::Advanced
    );
    assert_eq!(
        *left.requests.lock().unwrap(),
        [(0, 65_536), (65_536, 65_536)]
    );
    assert_eq!(
        *right.requests.lock().unwrap(),
        [(0, 65_536), (65_536, 65_536)]
    );
    assert_eq!(
        work.step(&sources, &cancel, || true).unwrap(),
        IdentityTurn::Complete(IdentityVerdict::IdenticalFiles)
    );
    for source in [&left, &right] {
        assert_eq!(
            *source.requests.lock().unwrap(),
            [(0, 65_536), (65_536, 65_536), (131_072, 65_536)]
        );
    }
}

fn finish_decoded(lines: &[Vec<String>; 2]) -> bool {
    let mut proof = DecodedText::default();
    for _ in 0..1000 {
        if let Some(equal) = proof.step(lines) {
            return equal;
        }
    }
    panic!("Decoded identity did not finish within its bounded test text");
}

#[test]
fn decoded_identity_is_exact_across_unicode_and_chunk_boundaries() {
    let lines = [vec!["漢".repeat(5000); 5], vec!["漢".repeat(5000); 5]];
    let mut proof = DecodedText::default();
    assert_eq!(
        proof.step(&lines),
        None,
        "A turn must not compare the entire 75 KiB"
    );
    assert!(finish_decoded(&lines));
    assert!(finish_decoded(&[
        vec!["ab\n".into(), "c".into()],
        vec!["a".into(), "b\nc".into()],
    ]));
}

#[test]
fn decoded_identity_finds_late_differences_and_preserves_line_endings() {
    let left = vec!["same\n".into(); 300];
    let mut right = left.clone();
    right.push("extra\n".into());
    let mut proof = DecodedText::default();
    assert_eq!(proof.step(&[left.clone(), right.clone()]), None);
    assert!(!finish_decoded(&[left, right]));
    assert!(!finish_decoded(&[vec!["a\r\n".into()], vec!["a\n".into()]]));
    assert!(!finish_decoded(&[vec!["a\n".into()], vec!["a".into()]]));
    assert!(finish_decoded(&[Vec::new(), Vec::new()]));
}
