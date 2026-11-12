use super::*;
use std::sync::{Mutex, atomic::AtomicBool};

type CheckedRead = (u64, u32, u64, Vec<u8>);

#[derive(Debug, Default)]
struct CheckedSource {
    calls: Mutex<Vec<CheckedRead>>,
    version: AtomicU64,
    validations: AtomicU64,
    bad_count: AtomicBool,
}
impl ContentSource for CheckedSource {
    fn length(&self) -> u64 {
        5
    }
    fn display_name(&self) -> String {
        "source".into()
    }
    fn version(&self) -> Vec<u8> {
        self.version.load(Ordering::Acquire).to_le_bytes().to_vec()
    }
    fn validate(&self) -> Result<(), Error> {
        self.validations.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
    fn read_at(&self, _: u64, _: u32) -> Result<Vec<u8>, Error> {
        panic!("a source with its own validated read must use that operation");
    }
    fn read_checked(
        &self,
        offset: u64,
        length: u32,
        expected_length: u64,
        expected_version: &[u8],
    ) -> Result<Vec<u8>, Error> {
        self.calls.lock().unwrap().push((
            offset,
            length,
            expected_length,
            expected_version.to_vec(),
        ));
        if expected_length != self.length() || expected_version != self.version() {
            return Err(changed());
        }
        if self.bad_count.load(Ordering::Acquire) {
            return Ok(vec![0; length as usize + 1]);
        }
        Ok(b"hello"[offset as usize..offset as usize + length as usize].to_vec())
    }
}

#[test]
fn broker_and_retained_reader_use_checked_hook_with_original_metadata() {
    let source = Arc::new(CheckedSource::default());
    let captured = CapturedFile::from_source(source.clone()).unwrap();
    source.validations.store(0, Ordering::Release);
    assert_eq!(captured.read_at(1, 3).unwrap(), b"ell");
    assert_eq!(source.validations.load(Ordering::Acquire), 0);
    let mut reader = captured.open().unwrap();
    assert_eq!(reader.read_at(3, 99).unwrap(), b"lo");
    assert_eq!(
        *source.calls.lock().unwrap(),
        vec![
            (1, 3, 5, 0u64.to_le_bytes().to_vec()),
            (3, 2, 5, 0u64.to_le_bytes().to_vec()),
        ]
    );
    source.version.store(1, Ordering::Release);
    assert!(captured.read_at(0, 1).is_err());
    assert!(reader.read_at(0, 1).is_err());
    assert!(captured.revalidate().is_err());
}

#[test]
fn override_cannot_bypass_bounds_or_counts_and_empty_reads_validate() {
    let source = Arc::new(CheckedSource::default());
    let captured = CapturedFile::from_source(source.clone()).unwrap();
    for (offset, length) in [(0, 0), (0, 256 * 1024 + 1), (u64::MAX, 1)] {
        assert!(captured.read_at(offset, length).is_err());
    }
    assert!(source.calls.lock().unwrap().is_empty());
    source.validations.store(0, Ordering::Release);
    assert!(captured.read_at(5, 1).unwrap().is_empty());
    assert!(captured.read_at(999, 1).unwrap().is_empty());
    assert_eq!(source.validations.load(Ordering::Acquire), 2);
    assert!(source.calls.lock().unwrap().is_empty());
    source.bad_count.store(true, Ordering::Release);
    assert!(captured.read_at(0, 1).is_err());
    source.version.store(1, Ordering::Release);
    assert!(captured.read_at(999, 1).is_err());
}

#[derive(Debug, Default)]
struct GenericSource {
    version: AtomicU64,
    length: AtomicU64,
    change_version: AtomicBool,
    change_length: AtomicBool,
}
impl ContentSource for GenericSource {
    fn length(&self) -> u64 {
        self.length.load(Ordering::Acquire)
    }
    fn display_name(&self) -> String {
        "generic".into()
    }
    fn version(&self) -> Vec<u8> {
        self.version.load(Ordering::Acquire).to_le_bytes().to_vec()
    }
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        if self.change_version.load(Ordering::Acquire) {
            self.version.fetch_add(1, Ordering::AcqRel);
        }
        if self.change_length.load(Ordering::Acquire) {
            self.length.fetch_add(1, Ordering::AcqRel);
        }
        Ok(b"hello"[offset as usize..offset as usize + length as usize].to_vec())
    }
}

#[test]
fn default_hook_rejects_mutation_during_read_for_both_entry_points() {
    for version in [true, false] {
        for retained in [true, false] {
            let source = Arc::new(GenericSource {
                length: AtomicU64::new(5),
                ..Default::default()
            });
            let captured = CapturedFile::from_source(source.clone()).unwrap();
            let mut reader = captured.open().unwrap();
            source.change_version.store(version, Ordering::Release);
            source.change_length.store(!version, Ordering::Release);
            let result = if retained {
                reader.read_at(0, 5)
            } else {
                captured.read_at(0, 5)
            };
            assert!(result.is_err(), "generic source mutation was accepted");
        }
    }
}
