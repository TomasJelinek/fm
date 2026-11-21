//! Backend-owned temporary content. Files never expose paths or handles to workers.
use crate::store::native::Root;
use std::{
    fs::File,
    io::{self, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
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

/// Bytes per checksummed block. `read_at` reads and checks only the blocks a read
/// touches, so a small read costs at most two blocks, and the checksums need 4 bytes per
/// block: 64 KiB of memory per GiB of content.
const BLOCK: u64 = 64 * 1024;

/// A writer of content without a byte ceiling (decoded archive members) queries its
/// spool volume at least once per this many appended bytes.
pub const SPACE_CHECK_INTERVAL: u64 = 64 << 20;

/// Free space that such writers leave on a spool volume of `total` bytes: the larger
/// of 1 GiB and 2% of its capacity.
pub fn free_space_reserve(total: u64) -> u64 {
    (1u64 << 30).max(total / 50)
}

/// Capacity of the volume that holds a private spool directory, in bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpoolSpace {
    /// Free space this process may still use: blocks available to unprivileged
    /// users on Linux, the caller's quota-limited free bytes on Windows.
    pub available: u64,
    /// The volume's capacity, also quota-limited for the caller on Windows.
    pub total: u64,
}

#[path = "spool_report.rs"]
mod report;
pub use report::{AppendSpool, SpoolPrefix};

#[path = "spool_root.rs"]
mod roots;
pub use roots::{ProcessRootGuard, SpoolRoot};

#[cfg(target_os = "linux")]
#[path = "spool_base_linux.rs"]
mod base;
#[cfg(target_os = "linux")]
pub use base::{effective_uid, resolve_base};

/// A verified private directory capability. Open and all I/O belong on backend workers.
/// Clones share the open directory and its count of created spools.
#[derive(Clone)]
pub struct PrivateSpoolDirectory {
    root: Arc<Root>,
    /// Spool files requested through any clone (`created_spools`).
    created: Arc<AtomicU64>,
}

/// Exclusive append ownership becomes shared read ownership after `finish` succeeds.
/// Dropping the last owner releases the OS file and its occupied storage, including
/// after an error. No permanent pathname, inherited handle or publication is provided.
pub struct Spool {
    file: File,
    // Keep ancestor pins alive even if the directory factory is dropped first.
    _root: Arc<Root>,
    length: u64,
    state: State,
    /// The CRC-32 of each `BLOCK` of appended bytes, in order: every complete block, and
    /// the last partial one once sealed (see `read_at`).
    blocks: Vec<u32>,
    /// The CRC-32 of the bytes appended after the last complete block.
    tail: crc32fast::Hasher,
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
        Root::open(path).map(|root| Self {
            root: Arc::new(root),
            created: Arc::default(),
        })
    }

    /// Create an empty file that the OS removes when its final handle closes.
    /// Filesystems without the required anonymous/deletion behavior fail explicitly.
    pub fn create(&self) -> io::Result<Spool> {
        Ok(Spool {
            file: self.create_file()?,
            _root: self.root.clone(),
            length: 0,
            state: State::Writing,
            blocks: Vec::new(),
            tail: crc32fast::Hasher::new(),
        })
    }

    /// Query the volume that holds this directory: one native call, for backend workers.
    pub fn space(&self) -> io::Result<SpoolSpace> {
        native::space(&self.root)
    }

    /// Spool files requested in this directory through any clone, including failed
    /// attempts. Each is a file-system operation that can wait for the volume's journal
    /// while other programs flush, so callers that avoid it can count on this.
    pub fn created_spools(&self) -> u64 {
        self.created.load(Ordering::Relaxed)
    }

    /// Whether `other` is a clone of this capability, sharing its open directory.
    pub fn is_same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.root, &other.root)
    }

    fn create_file(&self) -> io::Result<File> {
        self.created.fetch_add(1, Ordering::Relaxed);
        native::create(&self.root)
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
        // Each block's checksum covers exactly the bytes that the write accepted.
        let mut position = self.length;
        let mut rest = bytes;
        while !rest.is_empty() {
            let room = usize::try_from(BLOCK - position % BLOCK).unwrap_or(usize::MAX);
            let (now, later) = rest.split_at(room.min(rest.len()));
            self.tail.update(now);
            position += now.len() as u64;
            rest = later;
            if position.is_multiple_of(BLOCK) {
                self.blocks.push(std::mem::take(&mut self.tail).finalize());
            }
        }
        self.length = next;
        Ok(next)
    }

    /// Seal against further writes. This is not archive-integrity validation,
    /// publication or a durability guarantee, and the data is not flushed: the file is
    /// temporary, and synchronously writing back a large decoded archive stream made
    /// opening it wait seconds for the disk. `read_at` instead verifies every byte it
    /// returns against the CRC-32 recorded when it was appended, so a failed or lost
    /// write-back is reported as a read error, never returned as content.
    pub fn finish(&mut self) -> io::Result<()> {
        self.require(State::Writing)?;
        // The last block is complete now, however short.
        if !self.length.is_multiple_of(BLOCK) {
            self.blocks.push(std::mem::take(&mut self.tail).finalize());
        }
        self.state = State::Sealed;
        Ok(())
    }

    /// Read a bounded range from a sealed file. Concurrent reads use independent OS
    /// offsets, and EOF returns zero. Each `BLOCK` the range touches is read whole and
    /// checked against its CRC-32, and only checked bytes are returned; a mismatch or short
    /// read is an error.
    pub fn read_at(&self, offset: u64, output: &mut [u8]) -> io::Result<usize> {
        self.require(State::Sealed)?;
        checked_end(offset, output.len())?;
        let length = self.length.saturating_sub(offset).min(output.len() as u64) as usize;
        if length == 0 {
            return Ok(0);
        }
        let mut block = Vec::with_capacity(BLOCK as usize);
        let mut copied = 0;
        while copied < length {
            let at = offset + copied as u64;
            let index = at / BLOCK;
            let crc = usize::try_from(index)
                .ok()
                .and_then(|index| self.blocks.get(index))
                .copied()
                .ok_or_else(changed)?;
            let start = index * BLOCK;
            block.resize(((start + BLOCK).min(self.length) - start) as usize, 0);
            let mut filled = 0;
            while filled < block.len() {
                match native::read_at(&self.file, start + filled as u64, &mut block[filled..]) {
                    Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                    Ok(count) => filled += count,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(error) => return Err(error),
                }
            }
            if crc32fast::hash(&block) != crc {
                return Err(changed());
            }
            let from = (at - start) as usize;
            let count = (block.len() - from).min(length - copied);
            output[copied..copied + count].copy_from_slice(&block[from..from + count]);
            copied += count;
        }
        Ok(copied)
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
/// Not only another program's change: as nothing is flushed, a write-back that failed
/// (a full disk, an I/O error) shows as content that reads back differently.
fn changed() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Private spool content read back differs from what was written (the storage lost or \
         changed it)",
    )
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "Private spool offset or chunk is outside the supported range",
    )
}
