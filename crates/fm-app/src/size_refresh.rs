//! Batch size-only presentation updates independently of interactive events.
use std::time::Duration;

pub(crate) const INTERVAL: Duration = Duration::from_millis(250);

#[derive(Default)]
pub(crate) struct SizeRefresh {
    pending: bool,
}

impl SizeRefresh {
    pub(crate) fn changed(&mut self) {
        self.pending = true;
    }

    pub(crate) fn painted(&mut self) {
        self.pending = false;
    }

    pub(crate) fn take_pending(&mut self) -> bool {
        std::mem::take(&mut self.pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::platform::{Platform, PlatformError, WindowAdapter};
    use std::{cell::Cell, rc::Rc};

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
    fn size_flood_from_both_panes_paints_four_times_per_second() {
        let clock = Rc::new(Cell::new(Duration::ZERO));
        slint::platform::set_platform(Box::new(Clock(clock.clone()))).unwrap();
        let state = Rc::new(std::cell::RefCell::new(SizeRefresh::default()));
        let frames = Rc::new(Cell::new(0));
        let timer = slint::Timer::default();
        let observed = state.clone();
        let painted = frames.clone();
        timer.start(slint::TimerMode::Repeated, INTERVAL, move || {
            if observed.borrow_mut().take_pending() {
                painted.set(painted.get() + 1);
            }
        });
        for millis in 1..=1000 {
            // Concurrent scans share a single presentation pulse.
            state.borrow_mut().changed();
            state.borrow_mut().changed();
            clock.set(Duration::from_millis(millis));
            slint::platform::update_timers_and_animations();
        }
        assert_eq!(
            frames.get(),
            4,
            "size floods must not repaint on every event"
        );
        clock.set(Duration::from_millis(1250));
        slint::platform::update_timers_and_animations();
        assert_eq!(frames.get(), 4, "an idle timer does not repaint");
        state.borrow_mut().changed();
        state.borrow_mut().painted();
        clock.set(Duration::from_millis(1500));
        slint::platform::update_timers_and_animations();
        assert_eq!(
            frames.get(),
            4,
            "an immediate interactive repaint consumes pending sizes"
        );
        state.borrow_mut().changed();
        clock.set(Duration::from_millis(1750));
        slint::platform::update_timers_and_animations();
        assert_eq!(
            frames.get(),
            5,
            "a final update is delivered at the next pulse"
        );
    }
}
