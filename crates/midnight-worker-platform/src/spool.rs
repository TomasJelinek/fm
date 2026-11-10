//! Backend-owned temporary content. Files never expose paths or handles to workers.
use crate::store::native::Root;
use std::{
    fs::File,
    io::{self, Write},
    path::Path,
    sync::Arc,
};

#[cfg(target_os = "linux")]
#[path = "spool_linux.rs"]
mod native;

#[cfg(windows)]
#[path = "spool_windows.rs"]
mod native;
#[cfg(test)]
#[path = "spool_tests.rs"]
mod tests;

/// Maximum single read or append; total length is limited only by the OS and storage.
pub const MAX_IO: usize = 256 * 1024;

#[path = "spool_report.rs"]
mod report;
pub use report::{AppendSpool, SpoolPrefix};

/// A verified private directory capability. Open and all I/O belong on backend workers.
pub struct PrivateSpoolDirectory(Arc<Root>);

/// Exclusive append ownership becomes shared read ownership after `finish` succeeds.
/// Dropping the last owner releases the OS file and its occupied storage, including
/// after an error. No permanent pathname, inherited handle or publication is provided.
pub struct Spool {
    file: File,
    // Keep ancestor pins alive even if the directory factory is dropped first.
    _root: Arc<Root>,
    length: u64,
    state: State,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Writing,
    Sealed,
    Failed,
}

impl PrivateSpoolDirectory {
    /// Open or create a private root, refusing symlinks/reparse points and unsafe permissions.
    pub fn open(path: &Path) -> io::Result<Self> {
        Root::open(path).map(|root| Self(Arc::new(root)))
    }

    /// Create an empty file that the OS removes when its final handle closes.
    /// Filesystems without the required anonymous/deletion behavior fail explicitly.
    pub fn create(&self) -> io::Result<Spool> {
        Ok(Spool {
            file: native::create(&self.0)?,
            _root: self.0.clone(),
            length: 0,
            state: State::Writing,
        })
    }
}

impl Spool {
    /// Number of bytes successfully appended. A failed append makes the object unusable.
    pub fn length(&self) -> u64 {
        self.length
    }

    /// Append one nonempty bounded chunk at exactly the next offset.
    /// An actual I/O failure permanently prevents sealing/reading the file; invalid
    /// arguments do not poison it. There is no source/member/aggregate byte quota here.
    pub fn append(&mut self, expected_offset: u64, bytes: &[u8]) -> io::Result<u64> {
        self.append_with(expected_offset, bytes, |file, bytes| file.write_all(bytes))
    }

    fn append_with(
        &mut self,
        expected_offset: u64,
        bytes: &[u8],
        write: impl FnOnce(&mut File, &[u8]) -> io::Result<()>,
    ) -> io::Result<u64> {
        self.require(State::Writing)?;
        if bytes.is_empty() || expected_offset != self.length {
            return Err(invalid());
        }
        let next = checked_end(expected_offset, bytes.len())?;
        if let Err(error) = write(&mut self.file, bytes) {
            self.state = State::Failed;
            return Err(error);
        }
        self.length = next;
        Ok(next)
    }

    /// Flush file data and seal against further writes. This is not archive-integrity
    /// validation, publication or a persistent-cache durability guarantee.
    pub fn finish(&mut self) -> io::Result<()> {
        self.finish_with(File::sync_data)
    }

    fn finish_with(&mut self, flush: impl FnOnce(&File) -> io::Result<()>) -> io::Result<()> {
        self.require(State::Writing)?;
        if let Err(error) = flush(&self.file) {
            self.state = State::Failed;
            return Err(error);
        }
        self.state = State::Sealed;
        Ok(())
    }

    /// Read a bounded range from a sealed file. Concurrent reads use independent OS
    /// offsets, and EOF returns zero. Short reads and storage errors remain observable.
    pub fn read_at(&self, offset: u64, output: &mut [u8]) -> io::Result<usize> {
        self.require(State::Sealed)?;
        checked_end(offset, output.len())?;
        let length = self.length.saturating_sub(offset).min(output.len() as u64) as usize;
        if length == 0 {
            return Ok(0);
        }
        native::read_at(&self.file, offset, &mut output[..length])
    }

    fn require(&self, state: State) -> io::Result<()> {
        if self.state == state {
            Ok(())
        } else {
            Err(io::Error::other(match self.state {
                State::Writing => "Private spool is not sealed",
                State::Sealed => "Private spool is already sealed",
                State::Failed => "Private spool failed a previous I/O operation",
            }))
        }
    }
}

fn checked_end(offset: u64, length: usize) -> io::Result<u64> {
    if length > MAX_IO {
        return Err(invalid());
    }
    offset
        .checked_add(length as u64)
        .filter(|end| *end <= i64::MAX as u64)
        .ok_or_else(invalid)
}
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "Private spool offset or chunk is outside the supported range",
    )
}
