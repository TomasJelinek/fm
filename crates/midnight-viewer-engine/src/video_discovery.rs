//! The result of an expensive discovery that a service's windows share, such as the
//! installed video support: a success is kept, a failure is tried again later.
use std::{
    sync::{Mutex, PoisonError},
    time::{Duration, Instant},
};

/// A failed discovery is not tried again before this has passed: a package that was being
/// replaced, or a busy disk, recovers, while windows opened in a burst do not each repeat
/// a discovery that just failed.
const RETRY: Duration = Duration::from_secs(5);

/// One discovered value, found on demand by whichever caller needs it first.
pub(crate) struct Discovered<T> {
    state: Mutex<State<T>>,
}

enum State<T> {
    Unknown,
    Found(T),
    /// The last discovery failed at this time.
    Failed(Instant),
}

impl<T> Default for Discovered<T> {
    fn default() -> Self {
        Self {
            state: Mutex::new(State::Unknown),
        }
    }
}

impl<T: Clone> Discovered<T> {
    /// The value, running `discover` unless it was found before or failed less than
    /// `RETRY` before `now`. One discovery runs at a time; other callers wait for it and
    /// use its result. A success is kept for the service's lifetime.
    pub(crate) fn get(&self, now: Instant, discover: impl FnOnce() -> Option<T>) -> Option<T> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        match &*state {
            State::Found(value) => return Some(value.clone()),
            State::Failed(at) if now.saturating_duration_since(*at) < RETRY => return None,
            State::Unknown | State::Failed(_) => {}
        }
        let found = discover();
        *state = match &found {
            Some(value) => State::Found(value.clone()),
            None => State::Failed(now),
        };
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn a_failure_is_retried_after_a_while_and_a_success_is_kept() {
        let discovered = Discovered::<u32>::default();
        let runs = Cell::new(0);
        let start = Instant::now();
        let failing = || {
            runs.set(runs.get() + 1);
            None
        };
        assert_eq!(discovered.get(start, failing), None);
        // Windows opened right after it do not repeat the failed discovery.
        assert_eq!(discovered.get(start + RETRY / 2, failing), None);
        assert_eq!(runs.get(), 1);
        // A later window tries again, and the success is kept.
        let succeeding = || {
            runs.set(runs.get() + 1);
            Some(7)
        };
        assert_eq!(discovered.get(start + RETRY, succeeding), Some(7));
        assert_eq!(discovered.get(start + RETRY * 100, failing), Some(7));
        assert_eq!(runs.get(), 2);
    }

    #[test]
    fn a_panicking_discovery_leaves_the_next_caller_to_try_again() {
        let discovered = Discovered::<u32>::default();
        let now = Instant::now();
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            discovered.get(now, || panic!("discovery failed"))
        }));
        assert!(panicked.is_err());
        assert_eq!(discovered.get(now, || Some(3)), Some(3));
    }
}
