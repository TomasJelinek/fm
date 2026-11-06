//! A bounded presentation-only wheel impulse, independent of native input APIs.
use std::time::Duration;

pub(crate) const WHEEL_DURATION: Duration = Duration::from_millis(120);
pub(crate) const WHEEL_FRAME: Duration = Duration::from_millis(16);
pub(crate) const MAX_PENDING_PIXELS: f64 = 2400.0;

#[derive(Default)]
pub(crate) struct WheelMotion {
    updated: Option<Duration>,
    elapsed: Duration,
    distance: [f64; 2],
    delivered: [f64; 2],
}

impl WheelMotion {
    pub(crate) fn push(&mut self, x: f32, y: f32, now: Duration) -> bool {
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

    pub(crate) fn tick(&mut self, now: Duration) -> Option<(f32, f32)> {
        let updated = self.updated?;
        self.updated = Some(now.max(updated));
        // A delayed redraw must not spend animation time on pixels that were
        // never presented. Preserve the nominal duration at normal cadence and
        // extend it by missed frames rather than jumping whole rows.
        self.elapsed += now.saturating_sub(updated).min(WHEEL_FRAME);
        let fraction = (self.elapsed.as_secs_f64() / WHEEL_DURATION.as_secs_f64()).min(1.0);
        let eased = 1.0 - (1.0 - fraction) * (1.0 - fraction);
        let mut delta = [0.0; 2];
        for (axis, delta) in delta.iter_mut().enumerate() {
            let position = self.distance[axis] * eased;
            *delta = (position - self.delivered[axis]) as f32;
            self.delivered[axis] = position;
        }
        if fraction == 1.0 {
            self.updated = None;
        }
        (delta != [0.0; 2]).then_some((delta[0], delta[1]))
    }

    pub(crate) fn active(&self) -> bool {
        self.updated.is_some()
    }

    pub(crate) fn cancel(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finish(motion: &mut WheelMotion, after: Duration) -> (f32, f32) {
        let mut total = (0.0, 0.0);
        for frame in 1..=8 {
            if let Some(delta) = motion.tick(after + WHEEL_FRAME * frame) {
                total.0 += delta.0;
                total.1 += delta.1;
            }
        }
        assert!(!motion.active());
        total
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
        for millis in [32, 48, 64, 80, 96, 112, 120] {
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
        for millis in [32, 64, 96, 128, 160, 192, 224, 256] {
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
