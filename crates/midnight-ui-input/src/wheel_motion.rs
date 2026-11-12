//! A bounded presentation-only wheel impulse, independent of native input APIs.
use std::time::Duration;

pub const WHEEL_DURATION: Duration = Duration::from_millis(200);
// Wake high-refresh displays often enough; spend time only at a real redraw.
pub const WHEEL_FRAME: Duration = Duration::from_millis(8);
const MAX_FRAME_TIME: Duration = Duration::from_millis(16);
pub const MAX_PENDING_PIXELS: f64 = 2400.0;

#[derive(Default)]
pub struct WheelMotion {
    updated: Option<Duration>,
    elapsed: Duration,
    distance: [f64; 2],
    delivered: [f64; 2],
}

impl WheelMotion {
    pub fn push(&mut self, x: f32, y: f32, now: Duration) -> bool {
        if !x.is_finite() || !y.is_finite() || (x == 0.0 && y == 0.0) {
            return false;
        }
        for (axis, delta) in [f64::from(x), f64::from(y)].into_iter().enumerate() {
            let mut remaining = self.distance[axis] - self.delivered[axis];
            // A reverse notch expresses a new direction, not a request to finish
            // travelling the previous direction before returning.
            if delta != 0.0 && remaining.signum() != delta.signum() {
                remaining = 0.0;
            }
            self.distance[axis] =
                (remaining + delta).clamp(-MAX_PENDING_PIXELS, MAX_PENDING_PIXELS);
        }
        self.delivered = [0.0; 2];
        // Keep the last presentation timestamp while input is already active.
        // Repeated notches arriving just before redraw must not restart the
        // clock indefinitely and starve the queued movement.
        self.updated.get_or_insert(now);
        self.elapsed = Duration::ZERO;
        true
    }

    pub fn tick(&mut self, now: Duration) -> Option<(f32, f32)> {
        self.tick_bounded(now, [(f32::NEG_INFINITY, f32::INFINITY); 2])
    }

    /// Pixel/touchpad input already carries native momentum; add no second coast.
    pub fn push_direct(&mut self, x: f32, y: f32, now: Duration) -> bool {
        if !self.push(x, y, now) {
            return false;
        }
        self.elapsed = WHEEL_DURATION;
        true
    }

    /// Only deliver pixels covered by retained content. Missing guard pixels do
    /// not discard the target or finish an unseen animation.
    pub fn tick_bounded(&mut self, now: Duration, bounds: [(f32, f32); 2]) -> Option<(f32, f32)> {
        if bounds
            .iter()
            .any(|(low, high)| low.is_nan() || high.is_nan() || low > high)
        {
            return None;
        }
        let updated = self.updated?;
        self.updated = Some(now.max(updated));
        // A delayed redraw must not spend animation time on pixels that were
        // never presented. Preserve the nominal duration at normal cadence and
        // extend it by missed frames rather than jumping whole rows.
        self.elapsed += now.saturating_sub(updated).min(MAX_FRAME_TIME);
        let fraction = (self.elapsed.as_secs_f64() / WHEEL_DURATION.as_secs_f64()).min(1.0);
        let eased = 1.0 - (1.0 - fraction) * (1.0 - fraction);
        let mut delta = [0.0; 2];
        for (axis, delta) in delta.iter_mut().enumerate() {
            let position = self.distance[axis] * eased;
            let requested = position - self.delivered[axis];
            let delivered = requested.clamp(f64::from(bounds[axis].0), f64::from(bounds[axis].1));
            *delta = delivered as f32;
            self.delivered[axis] = if delivered == requested {
                position
            } else {
                self.delivered[axis] + delivered
            };
        }
        if fraction == 1.0 && self.distance == self.delivered {
            self.updated = None;
        }
        (delta != [0.0; 2]).then_some((delta[0], delta[1]))
    }

    /// Restart the remaining coast after new coverage arrives. Retain the last
    /// real redraw timestamp so the first covered frame can advance; tick still
    /// caps elapsed time and cannot spend an unseen pause in one large step.
    pub fn restart_remaining(&mut self) {
        self.distance = self.pending();
        self.delivered = [0.0; 2];
        self.elapsed = Duration::ZERO;
    }
    pub fn active(&self) -> bool {
        self.updated.is_some()
    }

    pub fn pending(&self) -> [f64; 2] {
        std::array::from_fn(|axis| self.distance[axis] - self.delivered[axis])
    }

    /// A known content edge terminates that axis without stopping the other.
    pub fn discard_axis(&mut self, axis: usize) {
        if axis >= 2 {
            return;
        }
        self.distance[axis] = self.delivered[axis];
        if self.distance == self.delivered {
            self.updated = None;
        }
    }

    pub fn cancel(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_guards_pause_delivery_without_discarding_motion_then_resume_in_bounded_steps() {
        let mut motion = WheelMotion::default();
        motion.push(0.0, 60.0, Duration::ZERO);
        for frame in 1..=30 {
            assert_eq!(
                motion.tick_bounded(WHEEL_FRAME * frame, [(0.0, 0.0); 2]),
                None
            );
        }
        assert!(motion.active());
        assert_eq!(motion.pending(), [0.0, 60.0]);
        let mut delivered = 0.0;
        for frame in 31..=42 {
            let (_, step) = motion
                .tick_bounded(WHEEL_FRAME * frame, [(0.0, 0.0), (0.0, 5.0)])
                .unwrap();
            assert!(step > 0.0 && step <= 5.0);
            delivered += step;
        }
        assert_eq!(delivered, 60.0);
        assert!(!motion.active());
    }

    #[test]
    fn native_pixel_input_has_no_second_tail_and_known_edges_stop_only_the_blocked_axis() {
        let mut motion = WheelMotion::default();
        motion.push_direct(3.5, -6.25, Duration::ZERO);
        assert_eq!(motion.tick(Duration::ZERO), Some((3.5, -6.25)));
        assert!(!motion.active());
        motion.push(20.0, 60.0, Duration::from_secs(1));
        for frame in 1..=30 {
            motion.tick_bounded(
                Duration::from_secs(1) + WHEEL_FRAME * frame,
                [(f32::NEG_INFINITY, f32::INFINITY), (0.0, 0.0)],
            );
        }
        assert_eq!(motion.pending(), [0.0, 60.0]);
        motion.discard_axis(1);
        assert!(!motion.active());
    }

    fn finish(motion: &mut WheelMotion, after: Duration) -> (f32, f32) {
        let mut total = (0.0, 0.0);
        for frame in 1..=26 {
            if let Some(delta) = motion.tick(after + WHEEL_FRAME * frame) {
                total.0 += delta.0;
                total.1 += delta.1;
            }
        }
        assert!(!motion.active());
        total
    }

    #[test]
    fn one_notch_has_small_presented_steps_and_a_short_continuous_tail() {
        let mut motion = WheelMotion::default();
        motion.push(0.0, -60.0, Duration::ZERO);
        let mut total = 0.0;
        let mut frames = 0;
        for frame in 1..=16 {
            if let Some((_, pixels)) = motion.tick(Duration::from_millis(frame * 16)) {
                assert!(
                    pixels.abs() < 11.0,
                    "visible frame jumps {pixels} logical pixels"
                );
                total += pixels;
                frames += 1;
            }
        }
        assert!(
            frames >= 12,
            "a wheel notch should span more than eight abrupt frames"
        );
        assert!(!motion.active(), "a single notch must finish within 256 ms");
        assert!((total + 60.0).abs() < 0.001);
    }

    #[test]
    fn wheel_notch_is_delivered_as_fractional_frames_with_exact_total() {
        let mut motion = WheelMotion::default();
        assert!(motion.push(0.0, -60.0, Duration::ZERO));
        assert!(motion.active());
        assert_eq!(motion.tick(Duration::ZERO), None);
        let first = motion.tick(Duration::from_millis(16)).unwrap();
        assert_eq!(first.0, 0.0);
        assert!(first.1 < -1.0 && first.1 > -20.0);
        let mut sum = first.1;
        for millis in (2..=13).map(|frame| (frame * 16).min(200)) {
            let frame = motion.tick(Duration::from_millis(millis)).unwrap();
            assert!(frame.1 < 0.0 && frame.1 > -20.0);
            sum += frame.1;
        }
        assert!((sum + 60.0).abs() < 0.001);
        assert!(!motion.active());
        assert_eq!(motion.tick(Duration::from_secs(1)), None);
    }

    #[test]
    fn delayed_presented_frames_do_not_fast_forward_invisible_pixels() {
        let mut motion = WheelMotion::default();
        motion.push(0.0, -60.0, Duration::ZERO);
        let mut total = 0.0;
        for millis in (1..=13).map(|frame| frame * 32) {
            let (_, pixels) = motion.tick(Duration::from_millis(millis)).unwrap();
            assert!(
                pixels < 0.0 && pixels > -20.0,
                "a delayed presentation still advances a fraction of a row: {pixels}"
            );
            total += pixels;
        }
        assert!((total + 60.0).abs() < 0.001);
        assert!(!motion.active());
    }

    #[test]
    fn same_direction_impulses_accumulate_remaining_pixels_without_replaying_delivery() {
        let mut motion = WheelMotion::default();
        motion.push(12.5, -60.0, Duration::ZERO);
        let first = motion.tick(Duration::from_millis(60)).unwrap();
        assert!(motion.push(4.0, -30.0, Duration::from_millis(70)));
        let rest = finish(&mut motion, Duration::from_millis(70));
        assert!((first.0 + rest.0 - 16.5).abs() < 0.001);
        assert!((first.1 + rest.1 + 90.0).abs() < 0.001);
        assert!(!motion.active());
    }

    #[test]
    fn sustained_input_just_before_redraw_cannot_starve_presented_motion() {
        let mut motion = WheelMotion::default();
        motion.push(0.0, -60.0, Duration::ZERO);
        let mut total = 0.0;
        for frame in 1..=30 {
            let now = Duration::from_millis(frame * 32);
            motion.push(0.0, -60.0, now);
            let (_, pixels) = motion
                .tick(now)
                .expect("existing motion advances on redraw");
            assert!(pixels < 0.0);
            total += pixels;
        }
        total += finish(&mut motion, Duration::from_millis(960)).1;
        assert!((total + 31.0 * 60.0).abs() < 0.001);
    }

    #[test]
    fn reversal_drops_old_residual_in_that_axis_and_changes_direction_immediately() {
        let mut motion = WheelMotion::default();
        motion.push(20.0, -60.0, Duration::ZERO);
        let first = motion.tick(Duration::from_millis(32)).unwrap();
        motion.push(0.0, 15.0, Duration::from_millis(40));
        let next = motion.tick(Duration::from_millis(56)).unwrap();
        assert!(next.1 > 0.0);
        let last = finish(&mut motion, Duration::from_millis(56));
        assert!((next.1 + last.1 - 15.0).abs() < 0.001);
        assert!((first.0 + next.0 + last.0 - 20.0).abs() < 0.001);
    }

    #[test]
    fn delayed_frames_preserve_pixels_and_cancel_discards_pending_motion() {
        let mut motion = WheelMotion::default();
        motion.push(0.0, -37.5, Duration::ZERO);
        let first = motion.tick(Duration::from_secs(2)).unwrap();
        assert!(first.1 < 0.0 && first.1 > -10.0);
        let rest = finish(&mut motion, Duration::from_secs(2));
        assert!((first.1 + rest.1 + 37.5).abs() < 0.001);
        assert_eq!(motion.tick(Duration::from_secs(3)), None);
        motion.push(0.0, -60.0, Duration::from_secs(3));
        motion.cancel();
        assert!(!motion.active());
        assert_eq!(motion.tick(Duration::from_secs(4)), None);
    }

    #[test]
    fn burst_residual_is_bounded_and_invalid_input_does_not_poison_an_existing_impulse() {
        let mut motion = WheelMotion::default();
        for _ in 0..10000 {
            assert!(motion.push(0.0, -60.0, Duration::ZERO));
        }
        assert!(!motion.push(f32::NAN, 1.0, Duration::ZERO));
        assert!(!motion.push(0.0, f32::INFINITY, Duration::ZERO));
        assert!(!motion.push(0.0, 0.0, Duration::ZERO));
        assert_eq!(
            finish(&mut motion, Duration::ZERO),
            (0.0, -(MAX_PENDING_PIXELS as f32))
        );
        assert!(!motion.active());
    }
}
