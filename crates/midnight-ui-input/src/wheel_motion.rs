//! A bounded presentation-only wheel glide (`glide.rs`), independent of native input APIs.
use crate::glide::{START_SLOPE, glide_duration, glide_scale, progress, progress_slope};
use std::time::Duration;

// Wake high-refresh displays often enough; spend time only at a real redraw.
pub const WHEEL_FRAME: Duration = Duration::from_millis(8);
/// A late redraw (a missed vsync, a slow frame) advances the glide by its real time, as
/// Chromium's wall-clock animation does, down to 20 frames a second. A longer stall resumes
/// from there instead of jumping by unseen pixels (the touchpad momentum's bound too).
const MAX_FRAME_TIME: Duration = Duration::from_millis(50);
/// The first frame of a glide restarted after a wait for content advances by at most one
/// 60 Hz frame: the wait was not a late frame, and the curve leaves at speed.
const RESTART_FRAME: Duration = Duration::from_nanos(16_666_667);
/// Only keeps the arithmetic finite: far beyond any content (a million 28 px rows are
/// 2.8e7 px), so no input's distance is ever dropped. Glide Off passes the same bound.
const MAX_PENDING_PIXELS: f64 = 1.0e9;
/// A notch glides over at most this many times the interval since the previous input
/// (times the glide scale): the faster a spin or a key repeat, the shorter each glide, so
/// the scroll keeps up with the input instead of trailing it by a whole glide.
const INTERVAL_RATIO: f64 = 2.0;
/// ... but over no less than this (times the glide scale), so notches that arrive
/// together (a busy event loop delivering queued input) still glide.
const MIN_GLIDE: Duration = Duration::from_millis(50);

/// One wheel glide that never scrolls slower than glide Off. Each notch adds its whole
/// distance to what is still to come, and the rest glides on the curve of `glide.rs` (it
/// leaves at once and eases out) from where the glide is at that moment, never slower than
/// it moves. It glides over the shortest of: Chromium's duration for that distance (200 ms
/// up to 120 px, down to 100 ms from 480 px) times [`glide_scale`]; [`INTERVAL_RATIO`]
/// times the interval since the previous input, times the scale; and the time the current
/// speed needs on the curve. A fast spin or a held key so builds speed and keeps up with
/// the input instead of trailing it. Time is the presented frames' time, each frame
/// counted up to [`MAX_FRAME_TIME`].
#[derive(Default)]
pub struct WheelMotion {
    updated: Option<Duration>,
    elapsed: Duration,
    duration: Duration,
    distance: [f64; 2],
    delivered: [f64; 2],
    /// When the previous input arrived.
    input: Option<Duration>,
    /// [`Self::restart_remaining`] restarted the glide; its next frame counts at most
    /// [`RESTART_FRAME`].
    restarted: bool,
}

/// The larger axis of `values`, with its sign.
fn major(values: [f64; 2]) -> f64 {
    if values[0].abs() > values[1].abs() {
        values[0]
    } else {
        values[1]
    }
}

impl WheelMotion {
    /// Add a notch's distance; it glides with this thread's [`glide_scale`].
    pub fn push(&mut self, x: f32, y: f32, now: Duration) -> bool {
        self.push_scaled(x, y, now, glide_scale())
    }

    /// [`Self::push`] with an explicit glide scale (0: the whole distance on the next frame).
    pub fn push_scaled(&mut self, x: f32, y: f32, now: Duration, scale: f32) -> bool {
        if !x.is_finite() || !y.is_finite() || (x == 0.0 && y == 0.0) {
            return false;
        }
        // Bring the glide in progress to this moment: the part of it due by now stays
        // owed to the next frame, and the rest glides on from here, so the curve's time is
        // the input's and the frames' alike (no time before the input counts twice).
        let (fraction, _) = self.progress_at(now);
        let eased = progress(fraction);
        // The speed of the glide in progress along its larger axis, pixels a second.
        let speed = if self.duration.is_zero() {
            0.0
        } else {
            major(self.distance) * progress_slope(fraction) / self.duration.as_secs_f64()
        };
        for (axis, delta) in [f64::from(x), f64::from(y)].into_iter().enumerate() {
            let position = if self.updated.is_some() {
                self.distance[axis] * eased
            } else {
                self.delivered[axis]
            };
            let mut remaining = self.distance[axis] - position;
            let mut owed = position - self.delivered[axis];
            // A reverse notch expresses a new direction, not a request to finish
            // travelling the previous direction before returning.
            if delta != 0.0 && (remaining + owed).signum() != delta.signum() {
                remaining = 0.0;
                owed = 0.0;
            }
            self.distance[axis] =
                (remaining + delta).clamp(-MAX_PENDING_PIXELS, MAX_PENDING_PIXELS);
            self.delivered[axis] = -owed;
        }
        self.updated = Some(self.updated.map_or(now, |updated| now.max(updated)));
        self.elapsed = Duration::ZERO;
        self.restarted = false;
        let previous = self.input.replace(now);
        let distance = major(self.distance);
        let mut duration = glide_duration(distance, scale);
        // Repeating input paces the glide, also where the previous glide already ended
        // (a short glide between key repeats); input after a pause is not paced.
        if let Some(previous) = previous {
            // As `glide_duration` reads the scale: not finite counts as 1.
            let scale = if scale.is_finite() {
                f64::from(scale.max(0.0))
            } else {
                1.0
            };
            let interval = (now.saturating_sub(previous).as_secs_f64() * INTERVAL_RATIO)
                .max(MIN_GLIDE.as_secs_f64());
            if let Ok(paced) = Duration::try_from_secs_f64(interval * scale) {
                duration = duration.min(paced);
            }
        }
        if speed != 0.0 && speed.signum() == distance.signum() && distance != 0.0 {
            // The curve leaves at START_SLOPE × distance / duration: at no less than the
            // current speed when the glide ends no later than this.
            if let Ok(keep) = Duration::try_from_secs_f64(START_SLOPE * distance / speed) {
                duration = duration.min(keep);
            }
        }
        self.duration = duration;
        true
    }

    /// The glide's time fraction (0 to 1) and elapsed time at `now`, counting the time
    /// since the last frame or input up to [`MAX_FRAME_TIME`] as a frame does.
    fn progress_at(&self, now: Duration) -> (f64, Duration) {
        let Some(updated) = self.updated else {
            return (1.0, self.elapsed);
        };
        let longest = if self.restarted {
            RESTART_FRAME
        } else {
            MAX_FRAME_TIME
        };
        let elapsed = self.elapsed + now.saturating_sub(updated).min(longest);
        let fraction = if self.duration.is_zero() {
            1.0
        } else {
            (elapsed.as_secs_f64() / self.duration.as_secs_f64()).min(1.0)
        };
        (fraction, elapsed)
    }

    /// The current speed along the larger axis, pixels a second (0 at rest), as of the
    /// last frame or input.
    #[cfg(test)]
    fn speed(&self) -> f64 {
        if self.updated.is_none() || self.duration.is_zero() {
            return 0.0;
        }
        let fraction = self.elapsed.as_secs_f64() / self.duration.as_secs_f64();
        major(self.distance) * progress_slope(fraction) / self.duration.as_secs_f64()
    }

    pub fn tick(&mut self, now: Duration) -> Option<(f32, f32)> {
        self.tick_bounded(now, [(f32::NEG_INFINITY, f32::INFINITY); 2])
    }

    /// Pixel/touchpad input already carries native momentum; add no second coast.
    pub fn push_direct(&mut self, x: f32, y: f32, now: Duration) -> bool {
        self.push_scaled(x, y, now, 0.0)
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
        // A delayed redraw advances by its real time, up to a bound: a long stall must
        // not spend animation time on pixels that were never presented.
        let (fraction, elapsed) = self.progress_at(now);
        self.updated = Some(now.max(updated));
        self.elapsed = elapsed;
        self.restarted = false;
        let eased = progress(fraction);
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
    /// real redraw timestamp so the first covered frame can advance; it advances by at
    /// most [`RESTART_FRAME`] and cannot spend an unseen pause in one large step.
    pub fn restart_remaining(&mut self) {
        self.distance = self.pending();
        self.delivered = [0.0; 2];
        self.elapsed = Duration::ZERO;
        self.duration = glide_duration(major(self.distance), glide_scale());
        self.restarted = true;
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
#[path = "wheel_motion_speed_tests.rs"]
mod speed_tests;

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
                // The fastest frame is the first: a sixth of the notch, not all of it.
                assert!(
                    pixels.abs() < 12.0,
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
        // The glide leaves at once, a sixth of the notch on the first 16 ms frame, where
        // Chromium's ease-in-out moved under 2 px of it, and every later frame moves less.
        let first = motion.tick(Duration::from_millis(16)).unwrap();
        assert_eq!(first.0, 0.0);
        assert!(first.1 < -9.0 && first.1 > -12.0, "{first:?}");
        let mut sum = first.1;
        let mut previous = first.1;
        for millis in (2..=13).map(|frame| (frame * 16).min(200)) {
            let frame = motion.tick(Duration::from_millis(millis)).unwrap();
            assert!(
                frame.1 < 0.0 && frame.1 >= previous,
                "{frame:?} after {previous}"
            );
            previous = frame.1;
            sum += frame.1;
        }
        assert!((sum + 60.0).abs() < 0.001);
        assert!(!motion.active());
        assert_eq!(motion.tick(Duration::from_secs(1)), None);
    }

    #[test]
    fn late_frames_advance_by_their_real_time_and_a_stall_by_at_most_fifty_ms() {
        // 30 Hz presentation: the notch still ends after its 200 ms, in seven frames.
        let mut motion = WheelMotion::default();
        motion.push(0.0, -60.0, Duration::ZERO);
        let mut total = 0.0;
        let mut frames = 0;
        while let Some((_, pixels)) = motion.tick(Duration::from_millis((frames + 1) * 32)) {
            assert!(pixels < 0.0 && pixels > -25.0, "{pixels}");
            total += pixels;
            frames += 1;
        }
        assert_eq!(frames, 7);
        assert!((total + 60.0).abs() < 0.001);
        // A stall of a second counts as 50 ms: a quarter of the glide (about half of its
        // distance), not all of it.
        motion.push(0.0, -60.0, Duration::from_secs(10));
        motion.tick(Duration::from_secs(10));
        let (_, first) = motion.tick(Duration::from_secs(11)).unwrap();
        let quarter = 60.0 * crate::glide::progress(0.25) as f32;
        assert!((first + quarter).abs() < 0.01 && quarter < 40.0, "{first}");
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
        // A first frame two seconds late moves the glide's first 50 ms, not all of it.
        let first = motion.tick(Duration::from_secs(2)).unwrap();
        assert!(first.1 < 0.0 && first.1 > -37.5 * 0.6, "{first:?}");
        let rest = finish(&mut motion, Duration::from_secs(2));
        assert!((first.1 + rest.1 + 37.5).abs() < 0.001);
        assert_eq!(motion.tick(Duration::from_secs(3)), None);
        motion.push(0.0, -60.0, Duration::from_secs(3));
        motion.cancel();
        assert!(!motion.active());
        assert_eq!(motion.tick(Duration::from_secs(4)), None);
    }

    /// A burst keeps every notch: the glide used to drop all but 2,400 px of what was
    /// still to come, so a held Page Down at Long or Very long covered half the distance.
    #[test]
    fn a_burst_keeps_every_notch_and_invalid_input_does_not_poison_an_existing_impulse() {
        let mut motion = WheelMotion::default();
        for _ in 0..10000 {
            assert!(motion.push(0.0, -60.0, Duration::ZERO));
        }
        assert!(!motion.push(f32::NAN, 1.0, Duration::ZERO));
        assert!(!motion.push(0.0, f32::INFINITY, Duration::ZERO));
        assert!(!motion.push(0.0, 0.0, Duration::ZERO));
        assert_eq!(motion.pending(), [0.0, -600_000.0]);
        let (x, y) = finish(&mut motion, Duration::ZERO);
        assert_eq!(x, 0.0);
        assert!((y + 600_000.0).abs() < 0.5, "{y}");
        assert!(!motion.active());
        // Only the arithmetic is bounded, far beyond any content.
        for _ in 0..3 {
            motion.push(0.0, f32::MAX, Duration::ZERO);
        }
        assert_eq!(motion.pending(), [0.0, MAX_PENDING_PIXELS]);
    }

    /// Presented 60 Hz frames until the glide ends: the frames' deltas and its duration.
    fn glide(motion: &mut WheelMotion, start: Duration) -> (Vec<f32>, Duration) {
        let frame = Duration::from_nanos(16_666_667);
        let mut steps = Vec::new();
        let mut now = start;
        while motion.active() && steps.len() < 200 {
            now += frame;
            if let Some((_, y)) = motion.tick(now) {
                steps.push(y);
            }
        }
        (steps, now - start)
    }

    #[test]
    fn one_notch_glides_for_chromiums_200_ms_at_60_hz_and_long_glides_are_shorter() {
        for (pixels, millis) in [(100.0, 200), (120.0, 200), (300.0, 150), (480.0, 100)] {
            let mut motion = WheelMotion::default();
            motion.push_scaled(0.0, pixels, Duration::ZERO, 1.0);
            motion.tick(Duration::ZERO);
            let (steps, took) = glide(&mut motion, Duration::ZERO);
            let total: f32 = steps.iter().sum();
            assert!((total - pixels).abs() < 0.01, "{pixels} px moved {total}");
            let frames = (f64::from(millis) * 60.0 / 1000.0).ceil() as u128;
            assert_eq!(
                took.as_nanos().div_ceil(16_666_667),
                frames,
                "{pixels} px took {took:?}, not {millis} ms"
            );
            // It leaves at once and eases out: the first frame moves the most.
            let peak = steps.iter().cloned().fold(0.0, f32::max);
            assert!(
                steps[0] == peak && *steps.last().unwrap() < peak / 10.0,
                "{steps:?}"
            );
        }
    }

    #[test]
    fn a_notch_during_a_glide_keeps_its_speed_for_chromiums_duration_or_sooner() {
        let frame = Duration::from_nanos(16_666_667);
        let mut motion = WheelMotion::default();
        motion.push_scaled(0.0, 100.0, Duration::ZERO, 1.0);
        let mut now = Duration::ZERO;
        let mut last = 0.0;
        for _ in 0..6 {
            now += frame;
            last = motion.tick(now).unwrap().1;
        }
        // The second notch at full speed: the next frame is not slower than the last.
        motion.push_scaled(0.0, 100.0, now, 1.0);
        let next = motion.tick(now + frame).unwrap().1;
        assert!(
            next >= last * 0.95,
            "the glide slowed from {last} to {next} px a frame"
        );
        let (_, rest) = glide(&mut motion, now + frame);
        assert!(motion.pending() == [0.0, 0.0]);
        // The remaining 112.5 px (12.5 px of the first notch and the second) glide for
        // Chromium's duration for that distance, 200 ms: the slower first notch's speed
        // reaches them later.
        let took = (rest + frame).as_secs_f64() * 1000.0;
        assert!((took - 200.0).abs() < 17.0, "{took} ms");

        // A small notch late in a fast glide keeps its speed and ends as soon as that speed
        // reaches the new target on the curve: 2.4 × 70 px at about 4,100 px/s, 40 ms, not
        // the 200 ms of a 70 px glide from rest.
        let mut motion = WheelMotion::default();
        motion.push_scaled(0.0, 480.0, Duration::ZERO, 1.0);
        let mut now = Duration::ZERO;
        for _ in 0..3 {
            now += frame;
            motion.tick(now);
        }
        motion.push_scaled(0.0, 10.0, now, 1.0);
        let (_, rest) = glide(&mut motion, now);
        assert!(rest <= Duration::from_millis(60), "{rest:?}");
    }

    #[test]
    fn a_restarted_glide_advances_one_frame_after_a_wait_for_content() {
        let mut motion = WheelMotion::default();
        motion.push_scaled(0.0, 60.0, Duration::ZERO, 1.0);
        // The content's edge holds the glide for a second without frames.
        motion.tick_bounded(Duration::from_millis(16), [(0.0, 0.0); 2]);
        motion.restart_remaining();
        let (_, first) = motion.tick(Duration::from_secs(1)).unwrap();
        // One 60 Hz frame of a fresh 200 ms glide of 60 px, not 50 ms of it.
        let frame = 60.0 * crate::glide::progress(1.0 / 12.0) as f32;
        assert!((first - frame).abs() < 0.01, "{first}");
        // Later frames count their real time again.
        let (_, second) = motion
            .tick(Duration::from_secs(1) + Duration::from_millis(40))
            .unwrap();
        let at = 60.0 * crate::glide::progress((16.666_667 + 40.0) / 200.0) as f32;
        assert!((first + second - at).abs() < 0.01, "{second}");
    }

    #[test]
    fn repeated_input_paces_the_glide_to_twice_its_interval_at_least_50_ms() {
        let frame = Duration::from_nanos(16_666_667);
        // Notches every two frames (33 ms) at Default: the last glides over 67 ms, so the
        // spin rests four frames after its last notch, not twelve.
        let mut motion = WheelMotion::default();
        let mut now = Duration::ZERO;
        for notch in 0..6 {
            if notch > 0 {
                now += frame;
                motion.tick(now);
                now += frame;
                motion.tick(now);
            }
            motion.push_scaled(0.0, 100.0, now, 1.0);
        }
        motion.tick(now);
        let (_, took) = glide(&mut motion, now);
        assert_eq!(took.as_nanos().div_ceil(16_666_667), 4, "{took:?}");
        // Two notches together glide no less than 50 ms; at Very long 200 ms.
        for (scale, frames) in [(1.0, 3), (4.0, 12)] {
            let mut motion = WheelMotion::default();
            motion.push_scaled(0.0, 100.0, Duration::ZERO, scale);
            motion.push_scaled(0.0, 100.0, Duration::ZERO, scale);
            motion.tick(Duration::ZERO);
            let (steps, took) = glide(&mut motion, Duration::ZERO);
            assert_eq!(took.as_nanos().div_ceil(16_666_667), frames, "x{scale}");
            assert!((steps.iter().sum::<f32>() - 200.0).abs() < 0.01);
        }
    }

    #[test]
    fn a_glide_scale_lengthens_the_glide_and_zero_moves_at_once() {
        let mut motion = WheelMotion::default();
        motion.push_scaled(0.0, 100.0, Duration::ZERO, 0.0);
        assert_eq!(motion.tick(Duration::ZERO), Some((0.0, 100.0)));
        assert!(!motion.active());
        // A notch after a pause (a second here) is not paced by the previous one.
        let later = Duration::from_secs(1);
        motion.push_scaled(0.0, 100.0, later, 2.0);
        motion.tick(later);
        let (_, took) = glide(&mut motion, later);
        assert_eq!(took.as_nanos().div_ceil(16_666_667), 24);
        // `push` reads this thread's scale.
        crate::set_glide_scale(0.0);
        motion.push(0.0, 50.0, Duration::ZERO);
        crate::set_glide_scale(1.0);
        assert_eq!(motion.tick(Duration::ZERO), Some((0.0, 50.0)));
    }
}
