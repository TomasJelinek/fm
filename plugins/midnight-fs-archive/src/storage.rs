//! Path-free staging interface. Production storage is a host broker; owned fixtures use memory.
use crate::*;
use midnight_filesystem_api::StagePurpose;
use std::{
    cell::RefCell,
    io::{Cursor, Seek, SeekFrom},
    rc::Rc,
};

pub trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}
pub type Reader = crate::window::Window;

/// Decoded-but-unstaged work is reported in batches. Source reads and appends
/// already renew the host watchdog, and each report is a pipe round trip.
const PROGRESS_STEP: u64 = 1024 * 1024;

pub trait Storage {
    fn begin(&self, purpose: StagePurpose, expected: Option<u64>, maximum: u64) -> io::Result<u64>;
    fn append(&self, stage: u64, offset: u64, bytes: &[u8]) -> io::Result<()>;
    fn seal(&self, stage: u64, length: u64) -> io::Result<()>;
    fn read(&self, stage: u64, offset: u64, length: u32) -> io::Result<Vec<u8>>;
    fn progress(&self, _stage: u64, _completed: u64) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Content {
    pub stage: u64,
    pub offset: u64,
    pub length: u64,
}

pub(crate) struct Stage {
    storage: Rc<dyn Storage>,
    id: u64,
    length: u64,
    expected: Option<u64>,
    maximum: u64,
    completed: u64,
    reported: u64,
    /// One transfer buffer per stage, reused by every member and solid sibling.
    buffer: Vec<u8>,
}
impl Stage {
    pub(crate) fn new(
        storage: Rc<dyn Storage>,
        purpose: StagePurpose,
        expected: Option<u64>,
        maximum: u64,
    ) -> Result<Self, Error> {
        let id = storage
            .begin(purpose, expected, maximum)
            .map_err(Error::storage)?;
        Ok(Self {
            storage,
            id,
            length: 0,
            expected,
            maximum,
            completed: 0,
            reported: 0,
            buffer: Vec::new(),
        })
    }
    pub(crate) fn append(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let end = self
            .length
            .checked_add(bytes.len() as u64)
            .ok_or_else(Error::limit)?;
        if end > self.maximum {
            // An indexed member that decodes past its declared length is corrupt;
            // only an unindexed scratch stream can exceed a budget.
            return Err(if self.expected.is_some() {
                length_mismatch()
            } else {
                Error::limit()
            });
        }
        if !bytes.is_empty() {
            self.storage
                .append(self.id, self.length, bytes)
                .map_err(Error::storage)?;
        }
        self.length = end;
        self.completed = self
            .completed
            .checked_add(bytes.len() as u64)
            .ok_or_else(Error::limit)?;
        // A completed append renews the host watchdog like a progress report.
        self.reported = self.completed;
        Ok(())
    }
    /// Stage everything `reader` yields, appending full transfer buffers.
    pub(crate) fn copy(&mut self, reader: impl Read) -> Result<u64, Error> {
        let start = self.length;
        self.transfer(reader, |stage, bytes| stage.append(bytes))?;
        Ok(self.length - start)
    }
    pub(crate) fn finish(self) -> Result<Content, Error> {
        if self
            .expected
            .is_some_and(|expected| expected != self.length)
        {
            return Err(length_mismatch());
        }
        self.storage
            .seal(self.id, self.length)
            .map_err(Error::storage)?;
        Ok(Content {
            stage: self.id,
            offset: 0,
            length: self.length,
        })
    }
    /// Drain a solid-block sibling that must be decoded for integrity only.
    pub(crate) fn discard(&mut self, reader: impl Read) -> Result<u64, Error> {
        let start = self.completed;
        self.transfer(reader, |stage, bytes| stage.advance(bytes.len()))?;
        Ok(self.completed - start)
    }
    /// Decode a solid-block sibling into memory. The caller publishes nothing from
    /// it before the whole block has been verified.
    pub(crate) fn retain(&mut self, reader: impl Read, length: u64) -> Result<Vec<u8>, Error> {
        let mut retained = Vec::with_capacity(usize::try_from(length).map_err(|_| Error::limit())?);
        self.transfer(reader, |stage, bytes| {
            if (retained.len() + bytes.len()) as u64 > length {
                return Err(length_mismatch());
            }
            retained.extend_from_slice(bytes);
            stage.advance(bytes.len())
        })?;
        Ok(retained)
    }
    fn advance(&mut self, count: usize) -> Result<(), Error> {
        self.completed = self
            .completed
            .checked_add(count as u64)
            .ok_or_else(Error::limit)?;
        if self.completed - self.reported >= PROGRESS_STEP {
            self.storage
                .progress(self.id, self.completed)
                .map_err(Error::storage)?;
            self.reported = self.completed;
        }
        Ok(())
    }
    /// Feed full transfer buffers to `sink`; decoders often return short reads, and
    /// every append or progress report is one host round trip.
    fn transfer(
        &mut self,
        mut reader: impl Read,
        mut sink: impl FnMut(&mut Self, &[u8]) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let mut buffer = std::mem::take(&mut self.buffer);
        buffer.resize(MAX_READ as usize, 0);
        let result = (|| {
            loop {
                let mut filled = 0;
                while filled < buffer.len() {
                    match reader.read(&mut buffer[filled..]) {
                        Ok(0) => break,
                        Ok(count) => filled += count,
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                        Err(error) => return Err(Error::from(error)),
                    }
                }
                if filled > 0 {
                    sink(self, &buffer[..filled])?;
                }
                if filled < buffer.len() {
                    return Ok(());
                }
            }
        })();
        self.buffer = buffer;
        result
    }
}
fn length_mismatch() -> Error {
    Error::new(
        ErrorCode::Corrupt,
        "Decoded length differs from archive index",
    )
}

pub(crate) struct StoredReader {
    storage: Rc<dyn Storage>,
    content: Content,
    position: u64,
}
impl StoredReader {
    pub(crate) fn new(storage: Rc<dyn Storage>, content: Content) -> Self {
        Self {
            storage,
            content,
            position: 0,
        }
    }
}
impl Read for StoredReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let count = (self.content.length - self.position)
            .min(output.len() as u64)
            .min(u64::from(MAX_READ)) as u32;
        if count == 0 {
            return Ok(0);
        }
        let data = self.storage.read(
            self.content.stage,
            self.content.offset + self.position,
            count,
        )?;
        if data.len() != count as usize {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        output[..data.len()].copy_from_slice(&data);
        self.position += data.len() as u64;
        Ok(data.len())
    }
}
impl Seek for StoredReader {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.position = seek_position(self.position, self.content.length, from)?;
        Ok(self.position)
    }
}
pub(crate) fn seek_position(position: u64, length: u64, from: SeekFrom) -> io::Result<u64> {
    let next = match from {
        SeekFrom::Start(next) => Some(next),
        SeekFrom::Current(delta) => position.checked_add_signed(delta),
        SeekFrom::End(delta) => length.checked_add_signed(delta),
    };
    next.filter(|next| *next <= length)
        .ok_or_else(|| io::ErrorKind::UnexpectedEof.into())
}

/// In-memory caller-owned fixture storage; the executable exclusively uses the host broker.
#[derive(Default)]
pub struct MemoryStorage(RefCell<Vec<(Vec<u8>, bool)>>);
impl Storage for MemoryStorage {
    fn begin(&self, _: StagePurpose, _: Option<u64>, _: u64) -> io::Result<u64> {
        let mut stages = self.0.borrow_mut();
        stages.push((Vec::new(), false));
        Ok(stages.len() as u64)
    }
    fn append(&self, stage: u64, offset: u64, bytes: &[u8]) -> io::Result<()> {
        let mut stages = self.0.borrow_mut();
        let (data, sealed) = stages
            .get_mut(stage as usize - 1)
            .ok_or(io::ErrorKind::InvalidData)?;
        if *sealed || offset != data.len() as u64 {
            return Err(io::ErrorKind::InvalidData.into());
        }
        data.extend_from_slice(bytes);
        Ok(())
    }
    fn seal(&self, stage: u64, length: u64) -> io::Result<()> {
        let mut stages = self.0.borrow_mut();
        let (data, sealed) = stages
            .get_mut(stage as usize - 1)
            .ok_or(io::ErrorKind::InvalidData)?;
        if *sealed || length != data.len() as u64 {
            return Err(io::ErrorKind::InvalidData.into());
        }
        *sealed = true;
        Ok(())
    }
    fn read(&self, stage: u64, offset: u64, length: u32) -> io::Result<Vec<u8>> {
        let stages = self.0.borrow();
        let (data, sealed) = stages
            .get(stage as usize - 1)
            .ok_or(io::ErrorKind::InvalidData)?;
        if !sealed {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let mut reader = Cursor::new(data);
        reader.set_position(offset);
        let mut bytes = vec![0; length as usize];
        let count = reader.read(&mut bytes)?;
        bytes.truncate(count);
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// Counts host round trips: appends with their sizes, and progress reports.
    #[derive(Default)]
    struct Counting {
        inner: MemoryStorage,
        appends: RefCell<Vec<usize>>,
        progress: Cell<usize>,
    }
    impl Storage for Counting {
        fn begin(
            &self,
            purpose: StagePurpose,
            expected: Option<u64>,
            maximum: u64,
        ) -> io::Result<u64> {
            self.inner.begin(purpose, expected, maximum)
        }
        fn append(&self, stage: u64, offset: u64, bytes: &[u8]) -> io::Result<()> {
            self.appends.borrow_mut().push(bytes.len());
            self.inner.append(stage, offset, bytes)
        }
        fn progress(&self, _: u64, _: u64) -> io::Result<()> {
            self.progress.set(self.progress.get() + 1);
            Ok(())
        }
        fn seal(&self, stage: u64, length: u64) -> io::Result<()> {
            self.inner.seal(stage, length)
        }
        fn read(&self, stage: u64, offset: u64, length: u32) -> io::Result<Vec<u8>> {
            self.inner.read(stage, offset, length)
        }
    }

    /// A decoder that returns at most `step` bytes per read.
    struct Trickle {
        remaining: usize,
        step: usize,
    }
    impl Read for Trickle {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            let count = self.remaining.min(self.step).min(output.len());
            output[..count].fill(7);
            self.remaining -= count;
            Ok(count)
        }
    }

    #[test]
    fn over_long_indexed_member_is_corrupt_but_unindexed_scratch_is_a_budget() {
        let storage: Rc<dyn Storage> = Rc::new(MemoryStorage::default());
        let mut member = Stage::new(
            storage.clone(),
            StagePurpose::Member { entry: 2 },
            Some(3),
            3,
        )
        .unwrap();
        assert_eq!(member.append(b"four").unwrap_err().code, ErrorCode::Corrupt);
        let mut scratch = Stage::new(storage, StagePurpose::Scratch, None, 3).unwrap();
        assert_eq!(scratch.append(b"four").unwrap_err().code, ErrorCode::Limit);
    }

    #[test]
    fn short_decoder_reads_fill_whole_appends_and_progress_is_batched() {
        let storage = Rc::new(Counting::default());
        let length = 3 * MAX_READ as usize + 10;
        let mut stage = Stage::new(
            storage.clone(),
            StagePurpose::Member { entry: 2 },
            Some(length as u64),
            length as u64,
        )
        .unwrap();
        let reader = Trickle {
            remaining: length,
            step: 1000,
        };
        assert_eq!(stage.copy(reader).unwrap(), length as u64);
        let full = MAX_READ as usize;
        assert_eq!(*storage.appends.borrow(), [full, full, full, 10]);
        // Ten MiB of discarded solid-block siblings: one report per MiB, not per read.
        let sibling = Trickle {
            remaining: 10 << 20,
            step: 1000,
        };
        assert_eq!(stage.discard(sibling).unwrap(), 10 << 20);
        assert_eq!(storage.progress.get(), 10);
        let retained = stage
            .retain(
                Trickle {
                    remaining: 5,
                    step: 2,
                },
                5,
            )
            .unwrap();
        assert_eq!(retained, [7; 5]);
        let longer = Trickle {
            remaining: 6,
            step: 6,
        };
        assert_eq!(
            stage.retain(longer, 5).unwrap_err().code,
            ErrorCode::Corrupt
        );
        stage.finish().unwrap();
    }
}
