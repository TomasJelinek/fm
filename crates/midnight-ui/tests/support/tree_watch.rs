//! Observe actual native subscriptions without replacing delivery or debounce policy.
//!
//! Besides counting native callbacks, the observer tracks which subscription batches
//! are still alive and when the last batch was registered. The engine drops a tree's
//! batch before registering its replacement and covers that gap with a logical rescan
//! (`tree.dirty()`), so a change made inside the gap legitimately produces no native
//! callback. Tests therefore fence on a live batch and accept either a fresh callback
//! or a re-registration as the native evidence for a change.
#![allow(dead_code)] // Each test module uses its own subset of these observations.
use midnight_core::*;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Instant,
};

#[derive(Default)]
struct Batches {
    next: u64,
    live: HashMap<u64, Vec<PathBuf>>,
    registered: usize,
    last_registration: Option<Instant>,
}

pub struct ObservedWatch {
    native: Arc<dyn WatchService>,
    callbacks: Arc<AtomicUsize>,
    batches: Arc<Mutex<Batches>>,
}

/// Keeps the native subscription and removes its batch from the live set on drop.
struct ObservedSubscription {
    _native: Box<dyn WatchSubscription>,
    batch: u64,
    batches: Arc<Mutex<Batches>>,
}

impl WatchSubscription for ObservedSubscription {}

impl Drop for ObservedSubscription {
    fn drop(&mut self) {
        let mut batches = self.batches.lock().unwrap_or_else(|e| e.into_inner());
        batches.live.remove(&self.batch);
    }
}

impl WatchService for ObservedWatch {
    fn watch_directory(
        &self,
        path: &Path,
        changed: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Box<dyn WatchSubscription>, FsError> {
        let count = self.callbacks.clone();
        self.native.watch_directory(
            path,
            Arc::new(move || {
                count.fetch_add(1, Ordering::SeqCst);
                changed();
            }),
        )
    }

    fn watch_directories(
        &self,
        paths: &[PathBuf],
        changed: Arc<dyn Fn(u32) + Send + Sync>,
    ) -> Result<Box<dyn WatchSubscription>, FsError> {
        let count = self.callbacks.clone();
        let subscription = self.native.watch_directories(
            paths,
            Arc::new(move |bits| {
                count.fetch_add(1, Ordering::SeqCst);
                changed(bits);
            }),
        )?;
        let mut batches = self.batches.lock().unwrap_or_else(|e| e.into_inner());
        let batch = batches.next;
        batches.next += 1;
        batches.live.insert(batch, paths.to_vec());
        batches.registered += 1;
        batches.last_registration = Some(Instant::now());
        drop(batches);
        Ok(Box::new(ObservedSubscription {
            _native: subscription,
            batch,
            batches: self.batches.clone(),
        }))
    }
}

impl ObservedWatch {
    pub fn new(native: Arc<dyn WatchService>) -> Self {
        Self {
            native,
            callbacks: Arc::new(AtomicUsize::new(0)),
            batches: Arc::default(),
        }
    }

    pub fn callbacks(&self) -> usize {
        self.callbacks.load(Ordering::SeqCst)
    }

    /// Number of batches registered so far (each re-registration counts).
    pub fn registrations(&self) -> usize {
        self.lock().registered
    }

    /// Snapshot for [`Self::native_evidence_since`], taken before an external change.
    pub fn mark(&self) -> (usize, usize) {
        (self.callbacks(), self.registrations())
    }

    /// A native callback arrived after `mark`, or a batch was registered after it:
    /// a change inside the drop-to-register gap is covered by the engine's rescan of
    /// the new registration instead of a callback.
    pub fn native_evidence_since(&self, mark: (usize, usize)) -> bool {
        self.callbacks() > mark.0 || self.registrations() > mark.1
    }

    /// Whether a currently live native subscription covers `path`.
    pub fn live_covers(&self, path: &Path) -> bool {
        self.lock()
            .live
            .values()
            .any(|paths| paths.iter().any(|p| p == path))
    }

    /// Short description for failure messages.
    pub fn describe(&self) -> String {
        let batches = self.lock();
        format!(
            "callbacks={}, registrations={}, live_batches={:?}, last_registration_age={:?}",
            self.callbacks(),
            batches.registered,
            batches.live.values().collect::<Vec<_>>(),
            batches.last_registration.map(|at| at.elapsed())
        )
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Batches> {
        self.batches.lock().unwrap_or_else(|e| e.into_inner())
    }
}
