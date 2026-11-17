//! On-demand file identities for Space/Backspace navigation.
//!
//! One backend actor per viewer window captures the entry its worker waits for, then
//! prefetches that entry's immediate neighbors. A cached identity is revalidated before it
//! is handed out, so a file changed after it was prefetched is captured again instead of
//! failing as changed. No other entry of the navigation order is captured.
use crate::capture::{CaptureEntry, Captured};
use midnight_plugin_host::CapturedFile;
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
};

type Wake = Arc<dyn Fn() -> bool + Send + Sync>;

const STOPPED: &str = "File identity preparation stopped.";

/// The worker's handle. Dropping it stops the actor after any in-flight capture returns.
pub(crate) struct Prefetch {
    shared: Arc<Shared>,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

#[derive(Default)]
struct State {
    /// The latest entry the worker waits for. A newer request supersedes it.
    wanted: Option<(u64, usize)>,
    /// A served request, retained until the worker takes it or asks for another entry.
    ready: Option<(u64, Captured)>,
    sequence: u64,
    stopped: bool,
    exited: bool,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }
}

impl Prefetch {
    /// Start the window's actor. The already captured focused entry seeds its cache.
    pub(crate) fn spawn(
        len: usize,
        current: usize,
        focused: Option<CapturedFile>,
        capture: CaptureEntry,
        closed: Arc<AtomicBool>,
        wake: Wake,
    ) -> Result<Self, String> {
        let shared = Arc::new(Shared::default());
        let exit = Exit {
            shared: shared.clone(),
            wake,
        };
        std::thread::Builder::new()
            .name("viewer-capture".into())
            .spawn(move || {
                let actor = Actor {
                    len,
                    capture,
                    closed,
                    cache: Vec::with_capacity(3),
                };
                actor.run(&exit, current, focused);
            })
            .map_err(|error| format!("Cannot prepare file identities: {error}"))?;
        Ok(Self { shared })
    }

    /// Ask for one entry's identity, superseding any earlier request.
    pub(crate) fn request(&self, index: usize) -> u64 {
        let (sequence, stale) = {
            let mut state = self.shared.lock();
            state.sequence = state.sequence.wrapping_add(1);
            state.wanted = Some((state.sequence, index));
            (state.sequence, state.ready.take())
        };
        drop(stale);
        self.shared.changed.notify_all();
        sequence
    }

    /// The result of request `sequence`, once it has been served.
    pub(crate) fn take(&self, sequence: u64) -> Option<Captured> {
        let mut state = self.shared.lock();
        if state
            .ready
            .as_ref()
            .is_some_and(|(ready, _)| *ready == sequence)
        {
            return state.ready.take().map(|(_, result)| result);
        }
        state.exited.then(|| Err(STOPPED.to_owned()))
    }
}

#[cfg(test)]
impl Prefetch {
    /// A handle without an actor: worker tests serve requests explicitly.
    pub(crate) fn manual() -> Self {
        Self {
            shared: Arc::default(),
        }
    }

    /// The entry of the latest unserved request.
    pub(crate) fn wanted(&self) -> Option<usize> {
        self.shared.lock().wanted.map(|(_, index)| index)
    }

    /// Serve the latest request as the actor would.
    pub(crate) fn serve(&self, result: Captured) {
        let mut state = self.shared.lock();
        if let Some((sequence, _)) = state.wanted.take() {
            state.ready = Some((sequence, result));
        }
    }
}

impl Drop for Prefetch {
    fn drop(&mut self) {
        let abandoned = {
            let mut state = self.shared.lock();
            state.stopped = true;
            state.wanted = None;
            state.ready.take()
        };
        drop(abandoned);
        self.shared.changed.notify_all();
    }
}

/// Marks the actor as exited, including after an unwind, so a waiting worker fails
/// its request instead of waiting forever.
struct Exit {
    shared: Arc<Shared>,
    wake: Wake,
}

impl Drop for Exit {
    fn drop(&mut self) {
        let waiting = {
            let mut state = self.shared.lock();
            state.exited = true;
            state.wanted.is_some() && !state.stopped
        };
        if waiting {
            (self.wake)();
        }
    }
}

enum Work {
    Wanted(u64, usize),
    Prefetch(usize),
}

struct Actor {
    len: usize,
    capture: CaptureEntry,
    closed: Arc<AtomicBool>,
    /// At most the last served entry and its two neighbors.
    cache: Vec<(usize, CapturedFile)>,
}

impl Actor {
    fn run(mut self, exit: &Exit, current: usize, focused: Option<CapturedFile>) {
        if let Some(file) = focused {
            self.cache.push((current, file));
        }
        let mut pending = self.neighbors(current);
        loop {
            let work = {
                let mut state = exit.shared.lock();
                loop {
                    if state.stopped || self.closed.load(Ordering::Acquire) {
                        return;
                    }
                    if let Some((sequence, index)) = state.wanted.take() {
                        break Work::Wanted(sequence, index);
                    }
                    if let Some(index) = pending.pop() {
                        break Work::Prefetch(index);
                    }
                    state = exit
                        .shared
                        .changed
                        .wait(state)
                        .unwrap_or_else(|error| error.into_inner());
                }
            };
            match work {
                Work::Wanted(sequence, index) => {
                    let result = self.serve(index);
                    let superseded = {
                        let mut state = exit.shared.lock();
                        if state.stopped {
                            return;
                        }
                        // A newer request owns the next result; this one is obsolete.
                        let superseded = state.wanted.is_some();
                        if !superseded {
                            state.ready = Some((sequence, result));
                        }
                        superseded
                    };
                    if !superseded {
                        (exit.wake)();
                    }
                    pending = self.neighbors(index);
                    self.cache
                        .retain(|(cached, _)| *cached == index || pending.contains(cached));
                }
                Work::Prefetch(index) => {
                    if self.cache.iter().all(|(cached, _)| *cached != index)
                        && let Ok(file) = self.fresh(index)
                    {
                        self.cache.push((index, file));
                    }
                }
            }
        }
    }

    /// Popped from the end: the next file first, then the previous one.
    fn neighbors(&self, index: usize) -> Vec<usize> {
        let mut neighbors: Vec<_> = [true, false]
            .into_iter()
            .filter_map(|backwards| crate::navigation::adjacent_file(index, self.len, backwards))
            .filter(|neighbor| *neighbor != index)
            .collect();
        neighbors.dedup();
        neighbors
    }

    fn serve(&mut self, index: usize) -> Captured {
        if let Some(position) = self.cache.iter().position(|(cached, _)| *cached == index) {
            let (_, file) = self.cache.swap_remove(position);
            if catch_unwind(AssertUnwindSafe(|| file.revalidate())).is_ok_and(|valid| valid.is_ok())
            {
                self.cache.push((index, file.clone()));
                return Ok(file);
            }
            // Modified, replaced or removed since it was cached: use its current identity.
        }
        let result = self.fresh(index);
        if let Ok(file) = &result {
            self.cache.push((index, file.clone()));
        }
        result
    }

    fn fresh(&self, index: usize) -> Captured {
        catch_unwind(AssertUnwindSafe(|| (self.capture)(index, &self.closed)))
            .unwrap_or_else(|_| Err("File identity preparation failed.".into()))
    }
}

#[cfg(test)]
#[path = "capture_prefetch_tests.rs"]
mod tests;
