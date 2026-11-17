//! Observe actual native subscriptions without replacing delivery or debounce policy.
//!
//! Native evidence is attributed to the directory it concerns, the way the engine
//! routes it: bit `i` of a batch callback names `paths[i]` (errors and overflow set
//! every active bit), and a flat `watch_directory` subscription names its one path.
//! A callback counts only while its subscription is live; a subscription being torn
//! down is no evidence.
//!
//! The engine drops a tree's batch before registering its replacement and covers that
//! gap with a logical rescan (`tree.dirty()`), so a change made inside the gap
//! legitimately produces no callback. Registering a batch that contains a directory is
//! therefore also evidence for that directory.
//!
//! Evidence for a directory can never come from another pane's subscription, from a
//! subscription that does not contain the directory, or from the engine's periodic
//! poll (which needs no subscription). A broken watch therefore fails its wait even
//! when that poll later reconciles the row.
#![allow(dead_code)] // Each test module uses its own subset of these observations.
use midnight_core::*;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::Instant,
};

type Observe = Arc<dyn Fn(u32) + Send + Sync>;

#[derive(Default)]
struct State {
    /// Orders evidence after a [`Mark`].
    sequence: u64,
    next_batch: u64,
    live: HashMap<u64, Batch>,
    callbacks: usize,
    registrations: usize,
    last_registration: Option<Instant>,
    /// Sequence number of the latest native evidence for each watched directory.
    evidence: HashMap<PathBuf, u64>,
}

struct Batch {
    paths: Arc<[PathBuf]>,
    /// The native registration has returned. Before that, a callback can already
    /// arrive from setup, but the subscription does not yet count as a fence.
    armed: bool,
}

/// A point in the observation sequence, taken before an external change.
#[derive(Clone, Copy, Debug)]
pub struct Mark(u64);

pub struct ObservedWatch {
    native: Arc<dyn WatchService>,
    state: Arc<Mutex<State>>,
}

/// Keeps the native subscription and removes its batch from the live set on drop,
/// before the native teardown, so callbacks during teardown are no evidence.
struct ObservedSubscription {
    batch: u64,
    state: Arc<Mutex<State>>,
    _native: Box<dyn WatchSubscription>,
}

impl WatchSubscription for ObservedSubscription {}

impl Drop for ObservedSubscription {
    fn drop(&mut self) {
        lock(&self.state).live.remove(&self.batch);
    }
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(|error| error.into_inner())
}

fn record_callback(state: &Mutex<State>, batch: u64, mask: u32) {
    let mut state = lock(state);
    state.callbacks += 1;
    let Some(paths) = state.live.get(&batch).map(|batch| batch.paths.clone()) else {
        return;
    };
    state.sequence += 1;
    let sequence = state.sequence;
    for (index, path) in paths.iter().enumerate().take(32) {
        if mask & (1 << index) != 0 {
            state.evidence.insert(path.clone(), sequence);
        }
    }
}

impl WatchService for ObservedWatch {
    fn watch_directory(
        &self,
        path: &Path,
        changed: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Box<dyn WatchSubscription>, FsError> {
        self.register(&[path.to_path_buf()], |observe| {
            self.native.watch_directory(
                path,
                Arc::new(move || {
                    observe(1);
                    changed();
                }),
            )
        })
    }

    fn watch_directories(
        &self,
        paths: &[PathBuf],
        changed: Arc<dyn Fn(u32) + Send + Sync>,
    ) -> Result<Box<dyn WatchSubscription>, FsError> {
        self.register(paths, |observe| {
            self.native.watch_directories(
                paths,
                Arc::new(move |mask| {
                    observe(mask);
                    changed(mask);
                }),
            )
        })
    }
}

impl ObservedWatch {
    pub fn new(native: Arc<dyn WatchService>) -> Self {
        Self {
            native,
            state: Arc::default(),
        }
    }

    /// Register one native subscription; `subscribe` must report each callback mask
    /// to the given observer before delivering it to the engine.
    fn register(
        &self,
        paths: &[PathBuf],
        subscribe: impl FnOnce(Observe) -> Result<Box<dyn WatchSubscription>, FsError>,
    ) -> Result<Box<dyn WatchSubscription>, FsError> {
        let paths: Arc<[PathBuf]> = paths.into();
        let batch = {
            let mut state = self.lock();
            let batch = state.next_batch;
            state.next_batch += 1;
            state.live.insert(
                batch,
                Batch {
                    paths: paths.clone(),
                    armed: false,
                },
            );
            batch
        };
        let observed = self.state.clone();
        let result = subscribe(Arc::new(move |mask| {
            record_callback(&observed, batch, mask);
        }));
        let mut state = self.lock();
        let native = match result {
            Ok(native) => native,
            Err(error) => {
                state.live.remove(&batch);
                return Err(error);
            }
        };
        state.registrations += 1;
        state.last_registration = Some(Instant::now());
        state.sequence += 1;
        let sequence = state.sequence;
        if let Some(batch) = state.live.get_mut(&batch) {
            batch.armed = true;
        }
        for path in paths.iter() {
            state.evidence.insert(path.clone(), sequence);
        }
        drop(state);
        Ok(Box::new(ObservedSubscription {
            batch,
            state: self.state.clone(),
            _native: native,
        }))
    }

    /// Snapshot for [`Self::native_evidence_for`], taken before an external change.
    pub fn mark(&self) -> Mark {
        Mark(self.lock().sequence)
    }

    /// After `mark`, a live subscription reported a change of `directory` (its bit,
    /// or every bit for an error or overflow), or a subscription containing
    /// `directory` was registered (the engine rescans the drop-to-register gap).
    pub fn native_evidence_for(&self, mark: Mark, directory: &Path) -> bool {
        self.lock()
            .evidence
            .get(directory)
            .is_some_and(|sequence| *sequence > mark.0)
    }

    /// Whether a registered, still live native subscription covers `path`.
    pub fn live_covers(&self, path: &Path) -> bool {
        self.lock()
            .live
            .values()
            .any(|batch| batch.armed && batch.paths.iter().any(|p| p == path))
    }

    /// Run a harness wait; if it fails, print these observations before unwinding.
    pub fn diagnosed<T>(&self, wait: impl FnOnce() -> T) -> T {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(wait)) {
            Ok(value) => value,
            Err(panic) => {
                eprintln!("native watch observations: {}", self.describe());
                std::panic::resume_unwind(panic)
            }
        }
    }

    /// Short description for failure messages.
    pub fn describe(&self) -> String {
        let state = self.lock();
        let mut live = state
            .live
            .iter()
            .map(|(id, batch)| (*id, batch.armed, batch.paths.to_vec()))
            .collect::<Vec<_>>();
        live.sort_by_key(|(id, ..)| *id);
        let mut evidence = state
            .evidence
            .iter()
            .map(|(path, sequence)| (*sequence, path.clone()))
            .collect::<Vec<_>>();
        evidence.sort();
        format!(
            "sequence={}, callbacks={}, registrations={}, last_registration_age={:?}, live_batches(id, armed, paths)={live:?}, latest_evidence(sequence, directory)={evidence:?}",
            state.sequence,
            state.callbacks,
            state.registrations,
            state.last_registration.map(|at| at.elapsed()),
        )
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        lock(&self.state)
    }
}
