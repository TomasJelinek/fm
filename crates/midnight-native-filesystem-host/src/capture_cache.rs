//! Many captured identities share a small backend reader cache instead of pinning
//! one native reader/mount for every file in the viewer's captured panel order.
use crate::{Cancel, NativeProvider, discovery::invalid};
use midnight_filesystem_plugin_api::Failure;
use midnight_plugin_host::{CapturedFile, ContentSource, Error};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

const READERS: usize = 16;

/// Backend-only snapshot capture with at most sixteen resident regular-file readers.
/// Reopening an evicted reader must match the original captured revision and length.
#[derive(Debug)]
pub struct CaptureCache {
    provider: NativeProvider,
    readers: Mutex<VecDeque<(u64, Arc<dyn ContentSource>)>>,
    next: AtomicU64,
}
impl CaptureCache {
    pub fn new(provider: NativeProvider) -> Arc<Self> {
        Arc::new(Self {
            provider,
            readers: Mutex::new(VecDeque::new()),
            next: AtomicU64::new(1),
        })
    }

    pub fn capture(
        self: &Arc<Self>,
        path: &Path,
        cancelled: Cancel<'_>,
    ) -> Result<CapturedFile, Failure> {
        let initial = self.provider.capture_source(path, cancelled)?;
        let id = self
            .next
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| invalid("Native capture identities exhausted"))?;
        let source = Arc::new(Snapshot {
            cache: self.clone(),
            id,
            path: path.into(),
            length: initial.length(),
            revision: initial.version(),
            name: initial.display_name(),
            initial: Mutex::new(Some(initial)),
        });
        // The constructor validates through this initial reader, but prefetching a
        // sibling must not displace the active viewer from the bounded reader cache.
        let captured = CapturedFile::from_source(source.clone()).map_err(invalid);
        let retired = lock(&source.initial).take();
        drop(retired);
        captured
    }

    fn get(&self, snapshot: &Snapshot) -> Result<Arc<dyn ContentSource>, Error> {
        {
            let mut readers = lock(&self.readers);
            if let Some(index) = readers.iter().position(|(id, _)| *id == snapshot.id)
                && let Some(entry) = readers.remove(index)
            {
                let source = entry.1.clone();
                readers.push_back(entry);
                return Ok(source);
            }
        }
        let source = self
            .provider
            .capture_source(&snapshot.path, &|| false)
            .map_err(|error| Error(error.to_string()))?;
        if source.length() != snapshot.length || source.version() != snapshot.revision {
            return Err(Error("Native file changed after capture".into()));
        }
        let mut retired = Vec::new();
        {
            let mut readers = lock(&self.readers);
            if let Some(index) = readers.iter().position(|(id, _)| *id == snapshot.id)
                && let Some(entry) = readers.remove(index)
            {
                retired.push(entry);
            }
            while readers.len() >= READERS {
                if let Some(entry) = readers.pop_front() {
                    retired.push(entry);
                }
            }
            readers.push_back((snapshot.id, source.clone()));
        }
        // Cleanup may call into the provider; never hold the cache mutex across it.
        drop(retired);
        Ok(source)
    }
}

#[derive(Debug)]
struct Snapshot {
    cache: Arc<CaptureCache>,
    id: u64,
    path: PathBuf,
    length: u64,
    revision: Vec<u8>,
    name: String,
    initial: Mutex<Option<Arc<dyn ContentSource>>>,
}
impl Snapshot {
    fn source(&self) -> Result<Arc<dyn ContentSource>, Error> {
        let initial = lock(&self.initial).clone();
        initial.map_or_else(|| self.cache.get(self), Ok)
    }
}
impl ContentSource for Snapshot {
    fn length(&self) -> u64 {
        self.length
    }
    fn version(&self) -> Vec<u8> {
        self.revision.clone()
    }
    fn display_name(&self) -> String {
        self.name.clone()
    }
    fn validate(&self) -> Result<(), Error> {
        let source = self.source()?;
        source.validate()?;
        if source.length() != self.length || source.version() != self.revision {
            return Err(Error("Native file changed after capture".into()));
        }
        Ok(())
    }
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        self.source()?.read_at(offset, length)
    }
    fn read_checked(
        &self,
        offset: u64,
        length: u32,
        expected_length: u64,
        expected_version: &[u8],
    ) -> Result<Vec<u8>, Error> {
        // Preserve the caller's original metadata even when source() reopens an
        // evicted reader; the native lease enforces it before its validated ReadAt.
        self.source()?
            .read_checked(offset, length, expected_length, expected_version)
    }
}
impl Drop for Snapshot {
    fn drop(&mut self) {
        let retired = {
            let mut readers = lock(&self.cache.readers);
            readers
                .iter()
                .position(|(id, _)| *id == self.id)
                .and_then(|index| readers.remove(index))
        };
        drop(retired);
    }
}
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
#[path = "capture_cache_tests.rs"]
mod tests;
