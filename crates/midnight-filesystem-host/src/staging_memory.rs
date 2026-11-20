//! Staged content kept in memory. Creating a spool file is a file-system operation that
//! waits for the volume's journal while other programs flush their writes (`fsync`):
//! seconds on a busy disk. A stream or member of at most `MEMORY_STAGE_LIMIT` bytes
//! therefore stays in memory while the process-wide `MEMORY_BUDGET` has room. Larger
//! content, and content arriving once the budget is used up, spills to an anonymous
//! spool file, whose reads stay CRC-verified (`Spool::read_at`).
use super::invalid;
use midnight_worker_platform::spool::{MAX_IO, PrivateSpoolDirectory, Spool};
use std::{
    io,
    sync::atomic::{AtomicU64, Ordering},
};

/// Largest stream or member that a stage keeps in memory.
pub(super) const MEMORY_STAGE_LIMIT: u64 = 16 << 20;
/// Staged bytes that every archive mount of this process may keep in memory together.
pub(super) static MEMORY_BUDGET: MemoryBudget = MemoryBudget::new(128 << 20);

/// A byte budget shared by every stage that keeps content in memory.
pub(crate) struct MemoryBudget {
    used: AtomicU64,
    limit: u64,
}

impl MemoryBudget {
    pub(crate) const fn new(limit: u64) -> Self {
        Self {
            used: AtomicU64::new(0),
            limit,
        }
    }

    /// Bytes held by in-memory stages and by the content sealed from them.
    #[cfg(test)]
    pub(crate) fn used(&self) -> u64 {
        self.used.load(Ordering::Acquire)
    }

    /// Take `bytes` only if the total stays within the limit, also under concurrent stages.
    fn take(&self, bytes: u64) -> bool {
        self.used
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|total| *total <= self.limit)
            })
            .is_ok()
    }
}

/// Budget bytes held by one in-memory stage. Dropping the stage or the last lease of
/// its sealed content returns them.
struct Reservation {
    budget: &'static MemoryBudget,
    bytes: u64,
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

/// Appended chunks in order, each with the offset it ends at, so growth never copies.
pub(super) struct Memory {
    chunks: Vec<(u64, Box<[u8]>)>,
    reservation: Reservation,
}

impl Memory {
    fn length(&self) -> u64 {
        self.chunks.last().map_or(0, |(end, _)| *end)
    }

    fn read_at(&self, offset: u64, output: &mut [u8]) -> usize {
        let mut index = self.chunks.partition_point(|(end, _)| *end <= offset);
        let mut copied = 0;
        while copied < output.len()
            && let Some((end, chunk)) = self.chunks.get(index)
        {
            let start = end - chunk.len() as u64;
            let from = (offset + copied as u64 - start) as usize;
            let count = (chunk.len() - from).min(output.len() - copied);
            output[copied..copied + count].copy_from_slice(&chunk[from..from + count]);
            copied += count;
            index += 1;
        }
        copied
    }
}

/// One stage's content: in memory first, in an anonymous spool file once it spills.
pub(super) enum Content {
    Memory(Memory),
    Disk(Spool),
}

impl Content {
    /// An empty stage in memory that draws on `budget`.
    pub(super) fn memory(budget: &'static MemoryBudget) -> Self {
        Self::Memory(Memory {
            chunks: Vec::new(),
            reservation: Reservation { budget, bytes: 0 },
        })
    }

    pub(super) fn length(&self) -> u64 {
        match self {
            Self::Memory(memory) => memory.length(),
            Self::Disk(spool) => spool.length(),
        }
    }

    /// Bytes that spilling now would write to a spool file before the next chunk.
    pub(super) fn in_memory(&self) -> u64 {
        match self {
            Self::Memory(memory) => memory.length(),
            Self::Disk(_) => 0,
        }
    }

    /// Append in memory while the stage stays within `limit` bytes and the budget has
    /// room. `None` leaves the chunk to the spool file (`spill`, then `append`).
    pub(super) fn append_in_memory(
        &mut self,
        offset: u64,
        bytes: &[u8],
        limit: u64,
    ) -> io::Result<Option<u64>> {
        let Self::Memory(memory) = self else {
            return Ok(None);
        };
        if offset != memory.length() || bytes.is_empty() || bytes.len() > MAX_IO {
            return Err(invalid());
        }
        let end = offset + bytes.len() as u64;
        if end > limit || !memory.reservation.budget.take(bytes.len() as u64) {
            return Ok(None);
        }
        memory.reservation.bytes += bytes.len() as u64;
        memory.chunks.push((end, bytes.into()));
        Ok(Some(end))
    }

    /// Move the content to a new spool file in `root`, returning its memory to the
    /// budget. Content already in a spool file stays where it is.
    pub(super) fn spill(&mut self, root: &PrivateSpoolDirectory) -> io::Result<()> {
        let Self::Memory(memory) = self else {
            return Ok(());
        };
        let mut spool = root.create()?;
        for (end, chunk) in &memory.chunks {
            let mut offset = end - chunk.len() as u64;
            for piece in chunk.chunks(MAX_IO) {
                offset = spool.append(offset, piece)?;
            }
        }
        *self = Self::Disk(spool);
        Ok(())
    }

    /// Append to the spool file of content that has spilled.
    pub(super) fn append(&mut self, offset: u64, bytes: &[u8]) -> io::Result<u64> {
        match self {
            Self::Disk(spool) => spool.append(offset, bytes),
            Self::Memory(_) => Err(invalid()),
        }
    }

    /// Seal against further appends; a spool file seals as `Spool::finish` describes.
    pub(super) fn finish(&mut self) -> io::Result<()> {
        match self {
            Self::Memory(_) => Ok(()),
            Self::Disk(spool) => spool.finish(),
        }
    }

    /// Read sealed content. Memory needs no checksum: only this process can change it.
    pub(super) fn read_at(&self, offset: u64, output: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Memory(memory) => Ok(memory.read_at(offset, output)),
            Self::Disk(spool) => spool.read_at(offset, output),
        }
    }

    #[cfg(test)]
    pub(super) fn on_disk(&self) -> bool {
        matches!(self, Self::Disk(_))
    }
}
