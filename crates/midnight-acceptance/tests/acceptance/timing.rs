//! Where a scenario's time goes: wall-clock time summed per named phase on the scenario's
//! own thread, reported as `timing` notes (`timing\t<phase>\t<seconds>\t<count>`). Cheap
//! enough to stay on: one `Instant` read at each end of a phase.
use std::{
    cell::RefCell,
    time::{Duration, Instant},
};

thread_local! {
    static PHASES: RefCell<Vec<(String, Duration, u32)>> = const { RefCell::new(Vec::new()) };
}

/// Add `elapsed` to `phase`.
pub fn add(phase: &str, elapsed: Duration) {
    PHASES.with(|phases| {
        let mut phases = phases.borrow_mut();
        match phases.iter_mut().find(|(name, _, _)| name == phase) {
            Some((_, total, count)) => {
                *total += elapsed;
                *count += 1;
            }
            None => phases.push((phase.to_owned(), elapsed, 1)),
        }
    });
}

/// Run `work` and add its time to `phase`.
pub fn time<T>(phase: &str, work: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let result = work();
    add(phase, started.elapsed());
    result
}

/// The phases so far as notes, in the order they first ran, and forget them.
pub fn take_notes() -> Vec<String> {
    PHASES.with(|phases| {
        phases
            .borrow_mut()
            .drain(..)
            .map(|(name, total, count)| {
                format!("timing\t{name}\t{:.1} s\t{count}", total.as_secs_f64())
            })
            .collect()
    })
}

/// Adds the time until it drops to its phase.
pub struct Span {
    phase: String,
    started: Instant,
}

impl Span {
    pub fn new(phase: impl Into<String>) -> Self {
        Self {
            phase: phase.into(),
            started: Instant::now(),
        }
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        add(&self.phase, self.started.elapsed());
    }
}
