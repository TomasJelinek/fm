//! A repeated UI timer that runs only while its tick reports remaining work.
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
    time::Duration,
};

pub(crate) struct Pulse {
    timer: slint::Timer,
    ticking: Cell<bool>,
    woken: Cell<bool>,
}

impl Pulse {
    /// Start ticking every `interval`. A tick returns whether work remains; after the
    /// first quiet tick the timer stops until [`Pulse::wake`].
    pub(crate) fn start(interval: Duration, mut tick: impl FnMut() -> bool + 'static) -> Rc<Self> {
        let pulse = Rc::new(Self {
            timer: slint::Timer::default(),
            ticking: Cell::new(false),
            woken: Cell::new(false),
        });
        let owner = Rc::downgrade(&pulse);
        pulse
            .timer
            .start(slint::TimerMode::Repeated, interval, move || {
                let Some(pulse) = owner.upgrade() else {
                    return;
                };
                pulse.ticking.set(true);
                pulse.woken.set(false);
                let busy = tick();
                pulse.ticking.set(false);
                // A wake from inside this tick (its own refresh, say) earns one more.
                if !busy && !pulse.woken.take() {
                    pulse.timer.stop();
                }
            });
        pulse
    }

    /// Tick within one interval. A running pulse is never postponed.
    pub(crate) fn wake(&self) {
        if self.ticking.get() {
            self.woken.set(true);
        } else if !self.timer.running() {
            self.timer.restart();
        }
    }

    pub(crate) fn stop(&self) {
        self.timer.stop();
    }

    #[cfg(test)]
    pub(crate) fn running(&self) -> bool {
        self.timer.running()
    }
}

/// Wake every live pulse of `pulses`.
pub(crate) fn wake_all(pulses: &[Weak<Pulse>]) {
    for pulse in pulses.iter().filter_map(Weak::upgrade) {
        pulse.wake();
    }
}

/// Refused command sends and unavailable owned scopes leave work behind. They set the
/// retry pass's flag and wake every watching pulse.
#[derive(Default)]
pub(crate) struct Refusals {
    refused: Cell<bool>,
    pulses: RefCell<Vec<Weak<Pulse>>>,
}

impl Refusals {
    pub(crate) fn watch(&self, pulse: Weak<Pulse>) {
        self.pulses.borrow_mut().push(pulse);
    }

    pub(crate) fn refused(&self) {
        self.refused.set(true);
        wake_all(&self.pulses.borrow());
    }

    /// Whether a send was refused since the last call.
    pub(crate) fn take(&self) -> bool {
        self.refused.replace(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::platform::{Platform, PlatformError, WindowAdapter};
    use std::cell::RefCell;

    struct Clock(Rc<Cell<Duration>>);
    impl Platform for Clock {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
            Err(PlatformError::NoPlatform)
        }

        fn duration_since_start(&self) -> Duration {
            self.0.get()
        }
    }

    const INTERVAL: Duration = Duration::from_millis(100);

    #[test]
    fn a_pulse_ticks_while_busy_stops_when_quiet_and_wakes_without_postponing() {
        let clock = Rc::new(Cell::new(Duration::ZERO));
        slint::platform::set_platform(Box::new(Clock(clock.clone()))).unwrap();
        let advance = |by: Duration| {
            clock.set(clock.get() + by);
            slint::platform::update_timers_and_animations();
        };
        let ticks = Rc::new(RefCell::new(Vec::new()));
        let busy = Rc::new(Cell::new(3_u32));
        let (recorded, remaining, now) = (ticks.clone(), busy.clone(), clock.clone());
        let pulse = Pulse::start(INTERVAL, move || {
            recorded.borrow_mut().push(now.get().as_millis());
            let work = remaining.get();
            remaining.set(work.saturating_sub(1));
            work > 0
        });
        for _ in 0..10 {
            advance(INTERVAL);
        }
        assert_eq!(
            *ticks.borrow(),
            [100, 200, 300, 400],
            "three busy ticks and one quiet tick"
        );
        assert!(!pulse.running());
        // Ten idle seconds cost nothing.
        for _ in 0..100 {
            advance(INTERVAL);
        }
        assert_eq!(ticks.borrow().len(), 4);

        // A wake runs one quiet tick one interval later.
        pulse.wake();
        advance(Duration::from_millis(60));
        assert_eq!(ticks.borrow().len(), 4);
        advance(Duration::from_millis(40));
        assert_eq!(ticks.borrow().last(), Some(&11_100));
        assert!(!pulse.running());

        // A wake while running never delays the next tick.
        busy.set(2);
        pulse.wake();
        advance(Duration::from_millis(100));
        advance(Duration::from_millis(50));
        pulse.wake();
        advance(Duration::from_millis(50));
        assert_eq!(&ticks.borrow()[5..], [11_200, 11_300]);
        advance(INTERVAL);
        advance(INTERVAL);
        assert_eq!(&ticks.borrow()[5..], [11_200, 11_300, 11_400]);
        assert!(!pulse.running());
    }

    #[test]
    fn a_wake_from_inside_a_quiet_tick_runs_one_more_tick() {
        let clock = Rc::new(Cell::new(Duration::ZERO));
        slint::platform::set_platform(Box::new(Clock(clock.clone()))).unwrap();
        let advance = |by: Duration| {
            clock.set(clock.get() + by);
            slint::platform::update_timers_and_animations();
        };
        let ticks = Rc::new(Cell::new(0));
        let slot: Rc<RefCell<Option<Rc<Pulse>>>> = Rc::default();
        let (counted, own) = (ticks.clone(), slot.clone());
        let pulse = Pulse::start(INTERVAL, move || {
            counted.set(counted.get() + 1);
            // The first tick's own presentation wakes it again; later ones do not.
            if counted.get() == 1
                && let Some(pulse) = own.borrow().as_ref()
            {
                pulse.wake();
            }
            false
        });
        *slot.borrow_mut() = Some(pulse.clone());
        for _ in 0..5 {
            advance(INTERVAL);
        }
        assert_eq!(ticks.get(), 2);
        assert!(!pulse.running());
        slot.borrow_mut().take();
    }

    #[test]
    fn a_refusal_wakes_every_watching_pulse_and_is_reported_once() {
        let clock = Rc::new(Cell::new(Duration::ZERO));
        slint::platform::set_platform(Box::new(Clock(clock.clone()))).unwrap();
        let pulses = [
            Pulse::start(INTERVAL, || false),
            Pulse::start(INTERVAL, || false),
        ];
        clock.set(INTERVAL);
        slint::platform::update_timers_and_animations();
        assert!(pulses.iter().all(|pulse| !pulse.running()));
        let refusals = Refusals::default();
        for pulse in &pulses {
            refusals.watch(Rc::downgrade(pulse));
        }
        assert!(!refusals.take());
        refusals.refused();
        assert!(pulses.iter().all(|pulse| pulse.running()));
        assert!(refusals.take());
        assert!(!refusals.take(), "a refusal is reported once");
    }
}
