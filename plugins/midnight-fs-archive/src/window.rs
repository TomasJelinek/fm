//! Seek-preserving read window over a brokered source.
//!
//! Every refill is one host round trip, so seeks inside the bytes already fetched
//! only move the cursor. `std::io::BufReader` discards its buffer on every seek,
//! which made ZIP local-header probes, TAR header walks and zstd frame peeks
//! refetch a whole 256 KiB window per entry.
use crate::storage::ReadSeek;
use midnight_filesystem_api::MAX_READ;
use std::io::{self, BufRead, Read, Seek, SeekFrom};

/// Largest window; equals the protocol's bounded source/spool read.
const MAX_WINDOW: usize = MAX_READ as usize;
/// Window used after a jump. It doubles while reads continue sequentially, so
/// scattered metadata probes do not each transfer a full maximum window.
const JUMP_WINDOW: usize = 16 * 1024;

pub struct Window {
    inner: Box<dyn ReadSeek>,
    length: u64,
    buffer: Vec<u8>,
    /// Source offset of `buffer[0]` and the number of valid bytes after it.
    start: u64,
    filled: usize,
    position: u64,
    /// Where the inner reader currently is, when known.
    inner_position: Option<u64>,
    /// End and size of the previous inner read; they classify the next one.
    last_end: Option<u64>,
    window: usize,
}

impl Window {
    /// `length` is the immutable source length; seeks beyond it fail as `UnexpectedEof`.
    pub fn new(inner: Box<dyn ReadSeek>, length: u64) -> Self {
        Self {
            inner,
            length,
            buffer: Vec::new(),
            start: 0,
            filled: 0,
            position: 0,
            inner_position: None,
            last_end: None,
            window: MAX_WINDOW,
        }
    }

    fn buffered(&self) -> Option<usize> {
        let offset = self.position.checked_sub(self.start)?;
        (offset < self.filled as u64).then_some(offset as usize)
    }

    /// The first read and sequential continuations grow the window; jumps restart small.
    fn next_window(&self) -> usize {
        match self.last_end {
            None => MAX_WINDOW,
            Some(end) if self.position >= end && self.position - end <= self.window as u64 => {
                self.window.saturating_mul(2).min(MAX_WINDOW)
            }
            Some(_) => JUMP_WINDOW,
        }
    }

    fn inner_read(&mut self, size: usize, direct: Option<&mut [u8]>) -> io::Result<usize> {
        // Until a seek and read succeed, the inner position is unknown: a failed or
        // interrupted call forces the next one to seek again.
        let known = self.inner_position.take();
        if known != Some(self.position) {
            self.inner.seek(SeekFrom::Start(self.position))?;
        }
        let count = match direct {
            Some(output) => self.inner.read(output)?,
            None => {
                // The refill overwrites the buffer from its start; forget the old range
                // first, so a read that fails part-way cannot serve mixed bytes later.
                self.filled = 0;
                if self.buffer.len() < size {
                    self.buffer.resize(size, 0);
                }
                let count = self.inner.read(&mut self.buffer[..size])?;
                self.start = self.position;
                self.filled = count;
                count
            }
        };
        let end = self
            .position
            .checked_add(count as u64)
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidData))?;
        self.inner_position = Some(end);
        self.last_end = Some(end);
        self.window = size;
        Ok(count)
    }
}

impl Read for Window {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        if self.buffered().is_none() {
            let size = self.next_window();
            if output.len() >= size {
                // A large request reads exactly what it needs, like `BufReader`.
                let request = output.len().min(MAX_WINDOW);
                let count = self.inner_read(request, Some(&mut output[..request]))?;
                self.position += count as u64;
                return Ok(count);
            }
        }
        let available = self.fill_buf()?;
        let count = available.len().min(output.len());
        output[..count].copy_from_slice(&available[..count]);
        self.consume(count);
        Ok(count)
    }
}

impl BufRead for Window {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        let offset = match self.buffered() {
            Some(offset) => offset,
            None => {
                let size = self.next_window();
                self.inner_read(size, None)?;
                0
            }
        };
        Ok(&self.buffer[offset..self.filled])
    }

    fn consume(&mut self, amount: usize) {
        let available = self
            .buffered()
            .map_or(0, |offset| self.filled - offset)
            .min(amount);
        self.position += available as u64;
    }
}

impl Seek for Window {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let target = match from {
            SeekFrom::Start(target) => Some(target),
            SeekFrom::Current(delta) => self.position.checked_add_signed(delta),
            SeekFrom::End(delta) => self.length.checked_add_signed(delta),
        }
        .filter(|target| *target <= self.length)
        .ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))?;
        // Fetched bytes stay valid: the brokered source is immutable for this mount.
        self.position = target;
        Ok(target)
    }

    fn stream_position(&mut self) -> io::Result<u64> {
        Ok(self.position)
    }
}

#[cfg(test)]
#[path = "window_tests.rs"]
mod tests;
