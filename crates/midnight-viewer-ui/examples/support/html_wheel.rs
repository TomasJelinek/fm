//! Reuse the probe's native, own-window wheel injection.
use midnight_viewer_ui::ViewerWindow;
use std::time::{Duration, Instant};

#[cfg(target_os = "windows")]
#[allow(dead_code)]
#[path = "../performance_probe/wheel_windows.rs"]
mod native;
pub(super) fn send(
    window: &ViewerWindow,
    delta: i16,
    timeline: &crate::timeline::Trace,
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        native::send(window, delta, timeline)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (window, delta, timeline);
        Err("native HTML wheel injection is Windows-only".into())
    }
}

const INPUTS: usize = 24;
const INTERVAL: Duration = Duration::from_millis(8);
const SETTLE: Duration = Duration::from_millis(300);

/// A bounded continuous wheel pass. Draws can coalesce multiple input messages.
pub(super) struct Burst {
    after_publication: u64,
    after_native_wheels: u64,
    sent: usize,
    draws: usize,
    first_input: Option<Instant>,
    last_input: Option<Instant>,
    first_delay: Option<Duration>,
    last_draw: Option<Instant>,
    maximum_gap: Duration,
    pub finished: bool,
}

impl Burst {
    pub fn new(after_publication: u64, after_native_wheels: u64) -> Self {
        Self {
            after_publication,
            after_native_wheels,
            sent: 0,
            draws: 0,
            first_input: None,
            last_input: None,
            first_delay: None,
            last_draw: None,
            maximum_gap: Duration::ZERO,
            finished: false,
        }
    }

    pub fn observe_publication(&mut self, publication: u64, native_wheels: u64) {
        // Reject pre-delivery pixels even when their redraw returns after delivery.
        // This ordering check does not associate later pixels with a specific input.
        if native_wheels <= self.after_native_wheels {
            self.after_publication = self.after_publication.max(publication);
        }
    }

    pub fn observe_draw(&mut self, publication: u64, native_wheels: u64) {
        self.observe_draw_at(publication, native_wheels, Instant::now());
    }

    fn observe_draw_at(&mut self, publication: u64, native_wheels: u64, now: Instant) {
        if native_wheels <= self.after_native_wheels || publication <= self.after_publication {
            return;
        }
        let Some(sent) = self.first_input else {
            return;
        };
        self.draws += 1;
        self.first_delay.get_or_insert(now.duration_since(sent));
        if let Some(previous) = self.last_draw.replace(now) {
            self.maximum_gap = self.maximum_gap.max(now.duration_since(previous));
        }
    }

    fn settled(&self, last_publication: Instant, now: Instant) -> bool {
        self.last_input
            .is_some_and(|last| now.duration_since(last) >= SETTLE)
            && now.duration_since(last_publication) >= SETTLE
            && self
                .last_draw
                .is_none_or(|returned| now.duration_since(returned) >= SETTLE)
    }

    pub fn advance(
        &mut self,
        window: &ViewerWindow,
        last_publication: Instant,
        timeline: &crate::timeline::Trace,
    ) -> Result<(), String> {
        if self.sent < INPUTS {
            if self
                .last_input
                .is_none_or(|sent| sent.elapsed() >= INTERVAL)
            {
                let sent = Instant::now();
                // Reverse every six quarter-notches to leave room on a short page.
                let delta = if (self.sent / 6).is_multiple_of(2) {
                    -30
                } else {
                    30
                };
                send(window, delta, timeline)?;
                self.first_input.get_or_insert(sent);
                self.last_input = Some(sent);
                self.sent += 1;
            }
            return Ok(());
        }
        let (Some(first), Some(last)) = (self.first_input, self.last_input) else {
            return Err("missing wheel burst input timestamps".into());
        };
        if !self.settled(last_publication, Instant::now()) {
            return Ok(());
        }
        let received = timeline
            .native_wheels()
            .saturating_sub(self.after_native_wheels);
        if received == 0 {
            return Err("wheel burst had no native delivery".into());
        }
        let Some(first_delay) = self.first_delay else {
            return Err("wheel burst produced no changed draw".into());
        };
        eprintln!(
            "burst posted={} received={received} changed_draws={} input_span_ms={:.3} first_draw_ms={:.3} max_draw_gap_ms={:.3}",
            self.sent,
            self.draws,
            last.duration_since(first).as_secs_f64() * 1000.0,
            first_delay.as_secs_f64() * 1000.0,
            self.maximum_gap.as_secs_f64() * 1000.0,
        );
        self.finished = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivery_gate_burst_rejects_pre_native_pixels_and_their_later_redraw() {
        let now = Instant::now();
        let mut burst = Burst::new(10, 5);
        burst.first_input = Some(now);
        burst.last_input = Some(now);
        burst.observe_publication(11, 5);
        burst.observe_draw_at(11, 5, now + Duration::from_millis(10));
        burst.observe_draw_at(11, 6, now + Duration::from_millis(20));
        assert_eq!(burst.draws, 0);
        assert!(burst.first_delay.is_none());
        burst.observe_publication(12, 6);
        burst.observe_draw_at(12, 6, now + Duration::from_millis(30));
        assert_eq!(burst.draws, 1);
        assert_eq!(burst.first_delay, Some(Duration::from_millis(30)));
    }

    #[test]
    fn delivery_gate_burst_settle_waits_since_changed_redraw_return() {
        let now = Instant::now();
        let mut burst = Burst::new(10, 5);
        burst.first_input = Some(now);
        burst.last_input = Some(now);
        burst.observe_draw_at(11, 6, now + Duration::from_millis(500));
        assert!(!burst.settled(now, now + Duration::from_millis(500)));
        assert!(!burst.settled(now, now + Duration::from_millis(799)));
        assert!(burst.settled(now, now + Duration::from_millis(800)));
    }
}
