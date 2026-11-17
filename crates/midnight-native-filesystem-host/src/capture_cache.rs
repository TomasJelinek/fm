//! Many captured identities (viewer, comparison, thumbnail, command-artwork and
//! archive captures) share a small backend reader cache instead of pinning one
//! native reader/mount for every captured file.
use crate::{Cancel, NativeProvider, client::PendingRelease, discovery::invalid};
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
        let initial = PendingRelease::new(self.provider.capture_source(path, cancelled)?);
        let id = match self
            .next
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            }) {
            Ok(id) => id,
            Err(_) => {
                release_source(initial.into_inner()).map_err(invalid)?;
                return Err(invalid("Native capture identities exhausted"));
            }
        };
        let source = Arc::new(Snapshot {
            cache: self.clone(),
            id,
            path: path.into(),
            length: initial.length(),
            revision: initial.version(),
            name: initial.display_name(),
            initial: Mutex::new(Some(initial.into_inner())),
            release_failure: Mutex::new(None),
            released: false,
        });
        // The constructor validates through this initial reader, but prefetching a
        // sibling must not displace the active viewer from the bounded reader cache.
        let captured = match CapturedFile::from_source(source.clone()) {
            Ok(captured) => captured,
            Err(error) => {
                release_source(source).map_err(invalid)?;
                return Err(invalid(error));
            }
        };
        let retired = lock(&source.initial).take();
        if let Some(retired) = retired
            && let Err(error) = release_source(retired)
        {
            source.note_release_failure(error.clone());
            // Consume both Snapshot references through their checked bridge.
            let _ = captured.release_on_worker();
            let _ = release_source(source);
            return Err(invalid(error));
        }
        Ok(captured)
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
        let mut unconfirmed = false;
        let source =
            match self
                .provider
                .capture_source_status(&snapshot.path, &|| false, &mut unconfirmed)
            {
                Ok(source) => source,
                Err(failure) => {
                    let error = Error(failure.to_string());
                    if unconfirmed {
                        snapshot.note_release_failure(error.clone());
                    }
                    return Err(error);
                }
            };
        let source = PendingRelease::new(source);
        if source.length() != snapshot.length || source.version() != snapshot.revision {
            if let Err(error) = release_source(source.into_inner()) {
                snapshot.note_release_failure(error.clone());
                return Err(error);
            }
            return Err(Error("Native file changed after capture".into()));
        }
        let mut retired = VecDeque::new();
        {
            let mut readers = lock(&self.readers);
            if let Some(index) = readers.iter().position(|(id, _)| *id == snapshot.id)
                && let Some(entry) = readers.remove(index)
            {
                retired.push_back(entry.1);
            }
            while readers.len() >= READERS {
                if let Some(entry) = readers.pop_front() {
                    retired.push_back(entry.1);
                }
            }
            readers.push_back((snapshot.id, Arc::clone(&source)));
        }
        // Cleanup may call into the provider; never hold the cache mutex across it.
        if let Err(error) = release_sources(retired) {
            snapshot.note_release_failure(error.clone());
            return Err(error);
        }
        Ok(source.into_inner())
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
    release_failure: Mutex<Option<Error>>,
    released: bool,
}
impl Snapshot {
    fn note_release_failure(&self, error: Error) {
        let mut failure = lock(&self.release_failure);
        if failure.is_none() {
            *failure = Some(error);
        }
    }
    fn source(&self) -> Result<Arc<dyn ContentSource>, Error> {
        if let Some(error) = lock(&self.release_failure).clone() {
            return Err(error);
        }
        let initial = lock(&self.initial).clone();
        initial.map_or_else(|| self.cache.get(self), Ok)
    }

    fn with_source<T>(
        &self,
        operation: impl FnOnce(&dyn ContentSource) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let source = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.source()))
        {
            Ok(result) => PendingRelease::new(result?),
            Err(panic) => {
                std::mem::forget(panic);
                let error =
                    Error("Native content acquisition panicked; teardown is unconfirmed".into());
                self.note_release_failure(error.clone());
                return Err(error);
            }
        };
        let result = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            operation(source.as_ref())
        })) {
            Ok(result) => result,
            Err(panic) => {
                std::mem::forget(panic);
                let error =
                    Error("Native content operation panicked; teardown is unconfirmed".into());
                self.note_release_failure(error.clone());
                return Err(error);
            }
        };
        // Observe the actual temporary source before consuming it. A cached
        // reader can be shared while its completed operation has already tainted
        // teardown; Snapshot's getter must not need a later cache traversal.
        if source.cleanup_unconfirmed() {
            self.note_release_failure(Error("Native content teardown is unconfirmed".into()));
        }
        if let Err(error) = release_source(source.into_inner()) {
            self.note_release_failure(error.clone());
            return Err(error);
        }
        result
    }

    fn detach_reader(&self) -> Option<Arc<dyn ContentSource>> {
        let mut readers = lock(&self.cache.readers);
        readers
            .iter()
            .position(|(id, _)| *id == self.id)
            .and_then(|index| readers.remove(index))
            .map(|(_, source)| source)
    }
}
impl ContentSource for Snapshot {
    fn cleanup_unconfirmed(&self) -> bool {
        lock(&self.release_failure).is_some()
    }

    fn release_on_worker(self: Arc<Self>) -> Result<(), Error> {
        if self.cleanup_unconfirmed() {
            std::mem::forget(self);
            return Err(Error("Native snapshot teardown is unconfirmed".into()));
        }
        let Some(snapshot) = Arc::into_inner(self) else {
            return Ok(());
        };
        let mut pending = PendingRelease::new(snapshot);
        if let Some(error) = lock(&pending.release_failure).clone() {
            return Err(error);
        }
        let initial = lock(&pending.initial).take();
        let cached = pending.detach_reader();
        pending.released = true;
        let mut sources = VecDeque::with_capacity(2);
        if let Some(source) = initial {
            sources.push_back(source);
        }
        if let Some(source) = cached {
            sources.push_back(source);
        }
        release_sources(sources)?;
        drop(pending.into_inner());
        Ok(())
    }
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
        self.with_source(|source| {
            source.validate()?;
            if source.length() != self.length || source.version() != self.revision {
                return Err(Error("Native file changed after capture".into()));
            }
            Ok(())
        })
    }
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        self.with_source(|source| source.read_at(offset, length))
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
        self.with_source(|source| {
            source.read_checked(offset, length, expected_length, expected_version)
        })
    }
}
impl Drop for Snapshot {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        let retired = self.detach_reader();
        drop(retired);
    }
}

fn release_source(source: Arc<dyn ContentSource>) -> Result<(), Error> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| source.release_on_worker())) {
        Ok(result) => result,
        Err(panic) => {
            std::mem::forget(panic);
            Err(Error(
                "Native content release panicked; teardown is unconfirmed".into(),
            ))
        }
    }
}
fn release_sources(sources: VecDeque<Arc<dyn ContentSource>>) -> Result<(), Error> {
    let mut pending = PendingRelease::new(sources);
    while let Some(source) = pending.pop_front() {
        release_source(source)?;
    }
    drop(pending.into_inner());
    Ok(())
}
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
#[path = "capture_cache_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "capture_cache_retirement_tests.rs"]
mod retirement_tests;
