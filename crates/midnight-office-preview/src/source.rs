use crate::{Cancellation, OfficeError};
use std::{
    io::{self, Read, Seek, SeekFrom},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

pub(crate) const MAX_WORK: u64 = 256 * 1024 * 1024;
pub(crate) const CHUNK: usize = 64 * 1024;

/// Captured identity/revision-validated bytes supplied by the host adapter.
/// Implementations must return at most `output.len()` bytes and must not reopen paths.
pub trait OfficeSource: Send + Sync {
    fn len(&self) -> u64;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn read_at(&self, offset: u64, output: &mut [u8], cancel: &Cancellation) -> io::Result<usize>;
}

pub(crate) struct Work {
    used: AtomicU64,
    cancel: Mutex<Cancellation>,
}
impl Work {
    pub fn new(cancel: &Cancellation) -> Arc<Self> {
        Arc::new(Self {
            used: AtomicU64::new(0),
            cancel: Mutex::new(cancel.clone()),
        })
    }
    pub fn begin(&self, cancel: &Cancellation) -> Result<(), OfficeError> {
        *self.cancel.lock().map_err(|_| OfficeError::Cancelled)? = cancel.clone();
        self.used.store(0, Ordering::Relaxed);
        self.check()
    }
    pub fn cancellation(&self) -> Result<Cancellation, OfficeError> {
        Ok(self
            .cancel
            .lock()
            .map_err(|_| OfficeError::Cancelled)?
            .clone())
    }
    pub fn check(&self) -> Result<(), OfficeError> {
        if self.cancellation()?.is_cancelled() {
            Err(OfficeError::Cancelled)
        } else {
            Ok(())
        }
    }
    pub fn reserve(&self, size: usize) -> Result<usize, OfficeError> {
        self.check()?;
        loop {
            let previous = self.used.load(Ordering::Relaxed);
            let count = (size as u64).min(MAX_WORK.saturating_sub(previous));
            if count == 0 && size != 0 {
                return Err(OfficeError::LimitExceeded(crate::model::WORK_LIMIT));
            }
            if self
                .used
                .compare_exchange_weak(
                    previous,
                    previous + count,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                return Ok(count as usize);
            }
        }
    }
    pub fn refund(&self, size: usize) {
        self.used.fetch_sub(size as u64, Ordering::Relaxed);
    }
}

pub(crate) struct CapturedReader {
    pub source: Arc<dyn OfficeSource>,
    pub length: u64,
    pub position: u64,
    pub work: Arc<Work>,
}
impl Read for CapturedReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.work.check().map_err(io::Error::other)?;
        let remaining = self.length.saturating_sub(self.position);
        let size = output.len().min(CHUNK).min(remaining as usize);
        if size == 0 {
            return Ok(0);
        }
        let cancel = self.work.cancellation().map_err(io::Error::other)?;
        let size = self.work.reserve(size).map_err(io::Error::other)?;
        let result = self
            .source
            .read_at(self.position, &mut output[..size], &cancel);
        let count = match result {
            Ok(count) => count,
            Err(error) => {
                self.work.refund(size);
                self.work.check().map_err(io::Error::other)?;
                return Err(error);
            }
        };
        if count > size {
            return Err(io::Error::other(OfficeError::Malformed("source overread")));
        }
        self.work.refund(size - count);
        self.work.check().map_err(io::Error::other)?;
        self.position += count as u64;
        Ok(count)
    }
}
impl Seek for CapturedReader {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.work.check().map_err(io::Error::other)?;
        let position = match from {
            SeekFrom::Start(offset) => i128::from(offset),
            SeekFrom::End(offset) => i128::from(self.length) + i128::from(offset),
            SeekFrom::Current(offset) => i128::from(self.position) + i128::from(offset),
        };
        if position < 0 || position > i128::from(self.length) {
            return Err(io::Error::other(OfficeError::Malformed(
                "ZIP seek outside source",
            )));
        }
        self.position = position as u64;
        Ok(self.position)
    }
}

pub(crate) struct Decoded<R> {
    pub inner: R,
    pub work: Arc<Work>,
}
impl<R: Read> Read for Decoded<R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.work.check().map_err(io::Error::other)?;
        let size = self
            .work
            .reserve(bytes.len().min(CHUNK))
            .map_err(io::Error::other)?;
        let count = match self.inner.read(&mut bytes[..size]) {
            Ok(count) => count,
            Err(error) => {
                self.work.refund(size);
                return Err(error);
            }
        };
        self.work.refund(size - count);
        self.work.check().map_err(io::Error::other)?;
        Ok(count)
    }
}
