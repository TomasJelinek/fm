//! Frozen read prefixes of a private append-only file; backend workers only.
use super::{PrivateSpoolDirectory, Root, checked_end, invalid, native};
use std::{
    fs::File,
    io::{self, Seek, SeekFrom, Write},
    sync::{Arc, Mutex},
};

/// Exclusive append ownership with immutable, independently retained read prefixes.
/// The private OS file disappears when the writer and last prefix are dropped.
pub struct AppendSpool {
    file: Arc<Mutex<File>>,
    root: Arc<Root>,
    length: u64,
    failed: bool,
}

#[derive(Clone)]
/// An immutable byte range. All reads belong on backend workers and remain bounded
/// by MAX_IO. Later appends cannot change this range or make its length grow.
pub struct SpoolPrefix {
    file: Arc<Mutex<File>>,
    _root: Arc<Root>,
    start: u64,
    length: u64,
}

impl PrivateSpoolDirectory {
    /// Create append-only private storage without changing ordinary sealed Spool semantics.
    pub fn create_append_only(&self) -> io::Result<AppendSpool> {
        Ok(AppendSpool {
            file: Arc::new(Mutex::new(native::create(&self.0)?)),
            root: self.0.clone(),
            length: 0,
            failed: false,
        })
    }
}

impl AppendSpool {
    /// Bytes successfully appended; a failed chunk never advances this boundary.
    pub fn length(&self) -> u64 {
        self.length
    }

    /// Append exactly at the current boundary. Actual I/O failure disables further
    /// writes; previously committed prefixes remain readable. No aggregate byte quota.
    pub fn append(&mut self, expected_offset: u64, bytes: &[u8]) -> io::Result<u64> {
        self.append_with(expected_offset, bytes, |file, bytes| file.write_all(bytes))
    }

    fn append_with(
        &mut self,
        expected_offset: u64,
        bytes: &[u8],
        write: impl FnOnce(&mut File, &[u8]) -> io::Result<()>,
    ) -> io::Result<u64> {
        if self.failed {
            return Err(io::Error::other("Append spool failed a previous write"));
        }
        if bytes.is_empty() || expected_offset != self.length {
            return Err(invalid());
        }
        let next = checked_end(expected_offset, bytes.len())?;
        let result = (|| {
            let mut file = self.file.lock().map_err(|_| poisoned())?;
            // Windows positioned reads may alter the handle's shared file pointer.
            // Serialize them with writes and restore the exact append position.
            file.seek(SeekFrom::Start(expected_offset))?;
            write(&mut file, bytes)
        })();
        if let Err(error) = result {
            self.failed = true;
            return Err(error);
        }
        self.length = next;
        Ok(next)
    }

    /// Freeze an already appended range. The caller may use an earlier complete-
    /// record boundary to exclude a partly written multi-chunk record after failure.
    pub fn prefix(&self, start: u64, end: u64) -> io::Result<SpoolPrefix> {
        if start > end || end > self.length {
            return Err(invalid());
        }
        Ok(SpoolPrefix {
            file: self.file.clone(),
            _root: self.root.clone(),
            start,
            length: end - start,
        })
    }
}
impl SpoolPrefix {
    pub fn length(&self) -> u64 {
        self.length
    }
    pub fn read_at(&self, offset: u64, output: &mut [u8]) -> io::Result<usize> {
        checked_end(offset, output.len())?;
        let count = self.length.saturating_sub(offset).min(output.len() as u64) as usize;
        if count == 0 {
            return Ok(0);
        }
        let file = self.file.lock().map_err(|_| poisoned())?;
        native::read_at(&file, self.start + offset, &mut output[..count])
    }
}

fn poisoned() -> io::Error {
    io::Error::other("Private append spool lock is poisoned")
}

#[cfg(test)]
#[path = "spool_report_tests.rs"]
mod tests;
