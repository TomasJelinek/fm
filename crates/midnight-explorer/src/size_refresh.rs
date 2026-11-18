//! Batch size-only presentation updates independently of interactive events.
use std::{
    cell::Cell,
    rc::{Rc, Weak},
    time::Duration,
};

pub(crate) const INTERVAL: Duration = Duration::from_millis(250);

/// One single-shot pulse armed by the first size-only change after a presentation.
pub(crate) struct SizeRefresh {
    pending: Cell<bool>,
    timer: slint::Timer,
    present: Box<dyn Fn()>,
}

impl SizeRefresh {
    pub(crate) fn new(present: impl Fn() + 'static) -> Rc<Self> {
        Rc::new(Self {
            pending: Cell::new(false),
            timer: slint::Timer::default(),
            present: Box::new(present),
        })
    }

    /// A size-only event changed cached totals: present them within one interval.
    pub(crate) fn changed(self: &Rc<Self>) {
        self.pending.set(true);
        if self.timer.running() {
            return;
        }
        let pulse: Weak<Self> = Rc::downgrade(self);
        self.timer
            .start(slint::TimerMode::SingleShot, INTERVAL, move || {
                if let Some(pulse) = pulse.upgrade()
                    && pulse.pending.replace(false)
                {
                    (pulse.present)();
                }
            });
    }

    /// An interactive refresh already presented the latest totals.
    pub(crate) fn painted(&self) {
        self.pending.set(false);
    }

    pub(crate) fn stop(&self) {
        self.timer.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::platform::{Platform, PlatformError, WindowAdapter};

    struct Clock(Rc<Cell<Duration>>);
    impl Platform for Clock {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
            Err(PlatformError::NoPlatform)
        }

        fn duration_since_start(&self) -> Duration {
            self.0.get()
        }
    }

    #[test]
    fn size_flood_from_both_panes_paints_four_times_per_second_and_idle_wakes_nothing() {
        let clock = Rc::new(Cell::new(Duration::ZERO));
        slint::platform::set_platform(Box::new(Clock(clock.clone()))).unwrap();
        let frames = Rc::new(Cell::new(0));
        let painted = frames.clone();
        let sizes = SizeRefresh::new(move || painted.set(painted.get() + 1));
        let advance = |to: u64| {
            clock.set(Duration::from_millis(to));
            slint::platform::update_timers_and_animations();
        };
        // Three changes within one interval share one presentation at its end.
        sizes.changed();
        advance(100);
        sizes.changed();
        sizes.changed();
        advance(249);
        assert_eq!(frames.get(), 0);
        advance(250);
        assert_eq!(frames.get(), 1);
        assert!(!sizes.timer.running(), "nothing pending arms no pulse");
        // Ten idle seconds fire nothing.
        for second in 1..=10 {
            advance(250 + second * 1000);
            assert!(!sizes.timer.running());
        }
        assert_eq!(frames.get(), 1);
        let start = 10_250;
        for millis in 1..=1000 {
            // Concurrent scans share a single presentation pulse.
            sizes.changed();
            sizes.changed();
            advance(start + millis);
        }
        assert_eq!(
            frames.get(),
            5,
            "size floods must not repaint on every event"
        );
        advance(start + 1250);
        assert_eq!(frames.get(), 5, "an idle pulse does not repaint");
        sizes.changed();
        sizes.painted();
        advance(start + 1500);
        assert_eq!(
            frames.get(),
            5,
            "an immediate interactive repaint consumes pending sizes"
        );
        sizes.changed();
        advance(start + 1750);
        assert_eq!(
            frames.get(),
            6,
            "a final update is delivered at the next pulse"
        );
    }
}
