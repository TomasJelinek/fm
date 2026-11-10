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
pub type Reader = std::io::BufReader<Box<dyn ReadSeek>>;

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
        })
    }
    pub(crate) fn append(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let end = self
            .length
            .checked_add(bytes.len() as u64)
            .ok_or_else(Error::limit)?;
        if end > self.maximum {
            return Err(Error::limit());
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
        Ok(())
    }
    pub(crate) fn copy(&mut self, mut reader: impl Read) -> Result<u64, Error> {
        let start = self.length;
        let mut bytes = vec![0; MAX_READ as usize];
        loop {
            let count = match reader.read(&mut bytes) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => result?,
            };
            if count == 0 {
                break;
            }
            self.append(&bytes[..count])?;
        }
        Ok(self.length - start)
    }
    pub(crate) fn finish(self) -> Result<Content, Error> {
        if self
            .expected
            .is_some_and(|expected| expected != self.length)
        {
            return Err(Error::new(
                ErrorCode::Corrupt,
                "Decoded length differs from archive index",
            ));
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
    pub(crate) fn discard(&mut self, mut reader: impl Read) -> Result<u64, Error> {
        let start = self.completed;
        let mut bytes = vec![0; MAX_READ as usize];
        loop {
            let count = match reader.read(&mut bytes) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                value => value?,
            };
            if count == 0 {
                return Ok(self.completed - start);
            }
            self.completed = self
                .completed
                .checked_add(count as u64)
                .ok_or_else(Error::limit)?;
            self.storage
                .progress(self.id, self.completed)
                .map_err(Error::storage)?;
        }
    }
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
