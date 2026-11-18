//! When the browser process's main thread pumps CEF next, under CEF's external message pump.
//! CEF asks for work through `OnScheduleMessagePumpWork`, from any thread; the main loop
//! waits on its command channel until the requested time, and pumps then.
use midnight_web_viewer_api as api;
use std::{
    sync::{
        Mutex, MutexGuard, OnceLock, PoisonError,
        atomic::{AtomicBool, Ordering},
        mpsc::SyncSender,
    },
    time::{Duration, Instant},
};

/// The longest CEF waits for work it requested, and the pump interval while it requests
/// none: cefclient's 30 Hz bound, which also bounds the delay of a request that was missed.
pub(crate) const MAX_DELAY: Duration = Duration::from_millis(33);

/// What the browser process's main loop receives.
pub(crate) enum Turn {
    Command(api::Command),
    /// CEF requested a pump; `Schedule` holds when.
    Pump,
    /// Standard input ended.
    End,
}

/// The pump deadline CEF requested. The deadline is the source of truth: a `Turn::Pump`
/// only interrupts the main loop's wait, so one lost on a full channel costs at most the
/// command batch that filled it.
pub(crate) struct Schedule {
    deadline: Mutex<Option<Instant>>,
    /// An interrupt was requested since the main loop's turn began.
    interrupt: AtomicBool,
    /// The main loop's channel, once it exists. CEF already requests work while it
    /// initializes; those requests only set the deadline.
    sender: OnceLock<SyncSender<Turn>>,
}

impl Schedule {
    /// The first pump is due at once.
    pub(crate) fn new(now: Instant) -> Self {
        Self {
            deadline: Mutex::new(Some(now)),
            interrupt: AtomicBool::new(false),
            sender: OnceLock::new(),
        }
    }

    fn deadline(&self) -> MutexGuard<'_, Option<Instant>> {
        self.deadline.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Interrupt the main loop's waits through `sender` from now on.
    pub(crate) fn connect(&self, sender: SyncSender<Turn>) {
        let _ = self.sender.set(sender);
    }

    /// CEF requests a pump after `delay_ms`, from any thread. The deadline, clamped to
    /// `0..=MAX_DELAY`, replaces a pending one, as CEF's contract asks; then the main
    /// loop's wait is interrupted unless an interrupt is already pending this turn.
    pub(crate) fn request(&self, delay_ms: i64, now: Instant) {
        let delay = u64::try_from(delay_ms).unwrap_or(0);
        let delay = Duration::from_millis(delay).min(MAX_DELAY);
        *self.deadline() = Some(now + delay);
        if !self.interrupt.swap(true, Ordering::AcqRel)
            && let Some(sender) = self.sender.get()
        {
            // A full channel already wakes the loop, whose next turn reads this deadline.
            let _ = sender.try_send(Turn::Pump);
        }
    }

    /// Begin a main-loop turn: clear the interrupt before reading the deadline, so a later
    /// request interrupts again, even one whose interrupt was lost on a full channel.
    pub(crate) fn turn(&self) {
        self.interrupt.swap(false, Ordering::AcqRel);
    }

    /// How long the main loop may wait for a command before the next pump is due.
    pub(crate) fn wait(&self, now: Instant) -> Duration {
        self.deadline().map_or(MAX_DELAY, |deadline| {
            deadline.saturating_duration_since(now)
        })
    }

    /// Whether the requested pump is due; a due deadline is consumed.
    pub(crate) fn due(&self, now: Instant) -> bool {
        let mut deadline = self.deadline();
        let due = deadline.is_some_and(|deadline| deadline <= now);
        if due {
            *deadline = None;
        }
        due
    }

    /// After a pump: unless CEF requested work meanwhile, or a requested pump is still
    /// pending, pump again after `MAX_DELAY`.
    pub(crate) fn pumped(&self, now: Instant) {
        self.deadline().get_or_insert(now + MAX_DELAY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn pumps(receiver: &mpsc::Receiver<Turn>) -> usize {
        receiver
            .try_iter()
            .filter(|turn| matches!(turn, Turn::Pump))
            .count()
    }

    #[test]
    fn requested_delays_are_clamped_and_replace_the_pending_deadline() {
        let now = Instant::now();
        let schedule = Schedule::new(now);
        assert_eq!(schedule.wait(now), Duration::ZERO, "the first pump is due");
        assert!(schedule.due(now));
        schedule.request(500, now);
        assert_eq!(schedule.wait(now), MAX_DELAY, "a long delay is clamped");
        schedule.request(5, now);
        assert_eq!(schedule.wait(now), Duration::from_millis(5), "replaced");
        schedule.request(20, now);
        assert_eq!(
            schedule.wait(now),
            Duration::from_millis(20),
            "a later request replaces an earlier deadline too"
        );
        for delay in [0, -1, i64::MIN] {
            schedule.request(delay, now);
            assert!(schedule.due(now), "a request of {delay} ms is due at once");
        }
        schedule.request(i64::MAX, now);
        assert_eq!(schedule.wait(now), MAX_DELAY);
        assert!(!schedule.due(now));
        assert!(schedule.due(now + MAX_DELAY));
        assert!(!schedule.due(now + MAX_DELAY), "a due deadline is consumed");
    }

    #[test]
    fn a_pump_without_a_new_request_falls_back_to_the_maximum_delay() {
        let now = Instant::now();
        let schedule = Schedule::new(now);
        assert!(schedule.due(now));
        schedule.pumped(now);
        assert_eq!(schedule.wait(now), MAX_DELAY);
        // Work CEF requests during a pump keeps its own deadline.
        assert!(schedule.due(now + MAX_DELAY));
        schedule.request(4, now);
        schedule.pumped(now);
        assert_eq!(schedule.wait(now), Duration::from_millis(4));
        // A pump for commands keeps the requested pump pending.
        schedule.request(10, now);
        schedule.pumped(now);
        assert_eq!(schedule.wait(now), Duration::from_millis(10));
    }

    #[test]
    fn requests_before_the_channel_exists_only_set_the_deadline() {
        let now = Instant::now();
        let schedule = Schedule::new(now);
        assert!(schedule.due(now));
        schedule.request(7, now);
        let (sender, receiver) = mpsc::sync_channel(4);
        schedule.connect(sender);
        assert_eq!(pumps(&receiver), 0);
        assert_eq!(schedule.wait(now), Duration::from_millis(7));
        schedule.turn();
        schedule.request(3, now);
        assert_eq!(pumps(&receiver), 1);
    }

    #[test]
    fn one_interrupt_per_turn_and_a_lost_one_cannot_suppress_a_later_request() {
        let now = Instant::now();
        let schedule = Schedule::new(now);
        let (sender, receiver) = mpsc::sync_channel(1);
        schedule.connect(sender.clone());
        schedule.request(10, now);
        schedule.request(2, now);
        assert_eq!(pumps(&receiver), 1, "the interrupt is coalesced");
        assert_eq!(
            schedule.wait(now),
            Duration::from_millis(2),
            "the deadline keeps the newest request"
        );
        // A command fills the channel, so the next interrupt is lost.
        schedule.turn();
        let command = api::Command {
            header: api::Header {
                session: 1,
                document: 1,
                sequence: 1,
            },
            kind: api::CommandKind::Copy,
        };
        sender.try_send(Turn::Command(command)).unwrap();
        schedule.request(1, now);
        assert!(matches!(receiver.try_recv(), Ok(Turn::Command(_))));
        assert!(receiver.try_recv().is_err(), "the interrupt was lost");
        // The turn that handles that command reads the deadline and clears the interrupt,
        // so a later request interrupts the following wait again.
        schedule.turn();
        assert_eq!(schedule.wait(now), Duration::from_millis(1));
        schedule.request(5, now);
        assert_eq!(pumps(&receiver), 1);
    }
}
