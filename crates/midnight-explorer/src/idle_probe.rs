//! Opt-in QA measurement of idle event-loop wakeups in a real desktop session.
//!
//! `MIDNIGHT_IDLE_WAKEUP_PROBE=<warmup_s>:<seconds>[:thumbnails]` starts the explorer with
//! in-memory default preferences (Thumbnails view on both panes with `thumbnails`), waits
//! `warmup_s` seconds after the event loop starts, counts winit wakeups by cause for
//! `seconds`, prints one line to stderr and quits the event loop. Without the variable
//! nothing is measured. Measure serially, with the pointer outside the window, in an
//! unlocked session: the reported timer lateness shows when the OS coalesced timers.
//! The explorer's application handler (`native_events`) forwards the loop events.
use slint::winit_030::winit::event::StartCause;
use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};

pub(crate) const VARIABLE: &str = "MIDNIGHT_IDLE_WAKEUP_PROBE";
/// A measurement longer than this is a typing error, not a QA run.
const MAXIMUM_SECONDS: u64 = 3600;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Config {
    pub(crate) warmup: Duration,
    pub(crate) window: Duration,
    pub(crate) thumbnails: bool,
}

pub(crate) fn from_env() -> Option<Config> {
    parse(std::env::var_os(VARIABLE)?.to_str()?)
}

fn parse(value: &str) -> Option<Config> {
    let mut fields = value.split(':');
    let seconds = |field: Option<&str>| {
        field?
            .parse::<u64>()
            .ok()
            .filter(|seconds| *seconds <= MAXIMUM_SECONDS)
    };
    let warmup = seconds(fields.next())?;
    let window = seconds(fields.next()).filter(|seconds| *seconds > 0)?;
    let thumbnails = match fields.next() {
        None => false,
        Some("thumbnails") => true,
        Some(_) => return None,
    };
    if fields.next().is_some() {
        return None;
    }
    Some(Config {
        warmup: Duration::from_secs(warmup),
        window: Duration::from_secs(window),
        thumbnails,
    })
}

#[derive(Default)]
struct Counts {
    wakeups: Cell<u64>,
    timers: Cell<u64>,
    events: Cell<u64>,
    /// How late timer wakeups were; large values mean the OS coalesced timers
    /// (for example while the session is locked), so the counts are not idle cost.
    lateness: Cell<Duration>,
    latest: Cell<Duration>,
}

impl Counts {
    fn snapshot(&self) -> (u64, u64, u64, Duration) {
        (
            self.wakeups.get(),
            self.timers.get(),
            self.events.get(),
            self.lateness.get(),
        )
    }
}

/// Counts every event-loop wakeup; the measurement is armed once the loop runs.
pub(crate) struct Probe {
    config: Config,
    counts: Rc<Counts>,
    armed: bool,
}

impl Probe {
    pub(crate) fn new(config: Config) -> Self {
        Self {
            config,
            counts: Rc::default(),
            armed: false,
        }
    }

    /// One winit wakeup; timer deadlines and cancelled waits (events, proxy wakes) are
    /// counted separately.
    pub(crate) fn wakeup(&self, cause: StartCause) {
        let counts = &self.counts;
        counts.wakeups.set(counts.wakeups.get() + 1);
        match cause {
            StartCause::ResumeTimeReached {
                requested_resume, ..
            } => {
                counts.timers.set(counts.timers.get() + 1);
                let late = Instant::now().saturating_duration_since(requested_resume);
                counts.lateness.set(counts.lateness.get() + late);
                counts.latest.set(counts.latest.get().max(late));
            }
            StartCause::WaitCancelled { .. } => counts.events.set(counts.events.get() + 1),
            StartCause::Poll | StartCause::Init => {}
        }
    }

    /// Arm the warmup and then the measurement once the event loop is running.
    pub(crate) fn arm(&mut self) {
        if std::mem::replace(&mut self.armed, true) {
            return;
        }
        let counts = self.counts.clone();
        let window = self.config.window;
        slint::Timer::single_shot(self.config.warmup, move || {
            let before = counts.snapshot();
            counts.latest.set(Duration::ZERO);
            let started = Instant::now();
            slint::Timer::single_shot(window, move || {
                let elapsed = started.elapsed().as_secs_f64();
                let after = counts.snapshot();
                // The wakeup that runs this timer ends the measurement; it is not idle work.
                let wakeups = (after.0 - before.0).saturating_sub(1);
                let timers = (after.1 - before.1).saturating_sub(1);
                let events = after.2 - before.2;
                let late = (after.3 - before.3).as_secs_f64() * 1000.0;
                eprintln!(
                    "idle_wakeups={wakeups} timer_wakeups={timers} event_wakeups={events} \
                     seconds={elapsed:.2} per_second={:.1} timer_lateness_ms_mean={:.1} \
                     timer_lateness_ms_max={:.1}",
                    wakeups as f64 / elapsed,
                    late / (after.1 - before.1).max(1) as f64,
                    counts.latest.get().as_secs_f64() * 1000.0
                );
                let _ = slint::quit_event_loop();
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_probe_variable_names_warmup_and_measurement_seconds() {
        assert_eq!(
            parse("15:60"),
            Some(Config {
                warmup: Duration::from_secs(15),
                window: Duration::from_secs(60),
                thumbnails: false,
            })
        );
        assert_eq!(
            parse("0:5:thumbnails"),
            Some(Config {
                warmup: Duration::ZERO,
                window: Duration::from_secs(5),
                thumbnails: true,
            })
        );
        for invalid in [
            "", "15", "15:", ":60", "15:0", "a:60", "15:b", "-1:60", "15:60:", "15:60:details",
            "15:60:thumbnails:1", "15:3601", "3601:60", " 15:60",
        ] {
            assert_eq!(parse(invalid), None, "{invalid:?}");
        }
    }
}
