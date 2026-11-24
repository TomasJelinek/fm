//! The wheel glide: how long a wheel notch's eased motion lasts (Chromium's
//! `cc/animation/scroll_offset_animation_curve.cc`, the `kInverseDelta` duration that
//! `ScrollType::kMouseWheel` selects) and the curve it follows, with one scale for the glide
//! and for touchpad momentum. Pure policy.
//!
//! The curve keeps the time axis of Chromium's ease-in-out (`kEaseInOutControlPoints`,
//! cubic-bezier(0.42, 0, 0.58, 1)) but not its ease-in: a glide leaves at once, at
//! [`START_SLOPE`] times its average speed, and eases out to rest (cubic-bezier(0.42, 1,
//! 0.58, 1)). Chromium's curve spends the first third of a glide almost still, so every
//! notch from rest, and every spin's first notches, trailed glide Off by most of the glide
//! (docs/verification/2026-11-24-scroll-glide-speed.md).
use std::cell::Cell;
use std::time::Duration;

/// Chromium's durations are counted in frames of 1/60 s (`kDurationDivisor`).
const FRAME_SECONDS: f64 = 1.0 / 60.0;
/// A glide of at most this many pixels lasts the longest (`kInverseDeltaRampStartPx`) ...
const RAMP_START: f64 = 120.0;
/// ... and one of at least this many the shortest (`kInverseDeltaRampEndPx`).
const RAMP_END: f64 = 480.0;
/// Frames of the longest and the shortest glide (`kInverseDeltaMaxDuration`, `...Min...`).
const LONGEST_FRAMES: f64 = 12.0;
const SHORTEST_FRAMES: f64 = 6.0;
/// The timing function's first and second control points' x (Chromium's
/// `kEaseInOutControlPoints`); both control points' y are 1.
const EASE_X1: f64 = 0.42;
const EASE_X2: f64 = 0.58;
/// The curve's slope at its start: a glide of `d` pixels over `t` seconds leaves at
/// `START_SLOPE * d / t` pixels a second (1 / [`EASE_X1`], about 2.4).
pub(crate) const START_SLOPE: f64 = 1.0 / EASE_X1;

thread_local! {
    /// The UI thread's glide scale; every window of the event loop shares it.
    static SCALE: Cell<f32> = const { Cell::new(1.0) };
}
/// The longest scale: eight times Chromium's glide.
const MAX_SCALE: f32 = 8.0;

/// The glide scale of this thread (the UI thread's event loop, which every window shares): 1.0 is Chromium's timing (the default), 0 moves wheel
/// notches at once and synthesises no touchpad momentum, larger values lengthen both. It
/// multiplies the glide's duration ([`glide_duration`]) and the momentum's decay time.
pub fn glide_scale() -> f32 {
    SCALE.with(Cell::get)
}

/// Set this thread's [`glide_scale`] for glides that start from now on (0 to 8; not
/// finite: 1).
pub fn set_glide_scale(scale: f32) {
    let scale = if scale.is_finite() {
        scale.clamp(0.0, MAX_SCALE)
    } else {
        1.0
    };
    SCALE.with(|cell| cell.set(scale));
}

/// How long Chromium glides a wheel scroll of `pixels` (the larger axis): 200 ms up to
/// 120 px, falling linearly to 100 ms at 480 px and beyond, times `scale`.
pub fn glide_duration(pixels: f64, scale: f32) -> Duration {
    let pixels = if pixels.is_finite() {
        pixels.abs()
    } else {
        0.0
    };
    let slope = (SHORTEST_FRAMES - LONGEST_FRAMES) / (RAMP_END - RAMP_START);
    let frames =
        (LONGEST_FRAMES + (pixels - RAMP_START) * slope).clamp(SHORTEST_FRAMES, LONGEST_FRAMES);
    let scale = if scale.is_finite() {
        f64::from(scale.max(0.0))
    } else {
        1.0
    };
    Duration::from_secs_f64(frames * FRAME_SECONDS * scale)
}

/// The cubic Bézier coordinate at parameter `t` of control values 0, `a`, `b`, 1.
fn bezier(t: f64, a: f64, b: f64) -> f64 {
    let u = 1.0 - t;
    3.0 * u * u * t * a + 3.0 * u * t * t * b + t * t * t
}

/// Its derivative with respect to `t`.
fn bezier_slope(t: f64, a: f64, b: f64) -> f64 {
    let u = 1.0 - t;
    3.0 * u * u * a + 6.0 * u * t * (b - a) + 3.0 * t * t * (1.0 - b)
}

/// The curve parameter whose x is `x`: Newton steps, bisection where they leave the range.
fn parameter(x: f64) -> f64 {
    let mut t = x;
    for _ in 0..8 {
        let error = bezier(t, EASE_X1, EASE_X2) - x;
        if error.abs() < 1e-9 {
            return t;
        }
        let slope = bezier_slope(t, EASE_X1, EASE_X2);
        if slope.abs() < 1e-9 {
            break;
        }
        t -= error / slope;
        if !(0.0..=1.0).contains(&t) {
            break;
        }
    }
    let (mut low, mut high) = (0.0, 1.0);
    t = x;
    for _ in 0..48 {
        let value = bezier(t, EASE_X1, EASE_X2);
        if (value - x).abs() < 1e-9 {
            break;
        }
        if value < x {
            low = t;
        } else {
            high = t;
        }
        t = (low + high) / 2.0;
    }
    t
}

/// The glide's progress (0 to 1) at time fraction `fraction` of its duration: the timing
/// function cubic-bezier(0.42, 1, 0.58, 1), whose progress is `1 - (1 - t)^3` at curve
/// parameter `t`. It leaves at [`START_SLOPE`] and eases out to rest, never overshooting
/// or turning back.
pub(crate) fn progress(fraction: f64) -> f64 {
    if fraction <= 0.0 {
        return 0.0;
    }
    if fraction >= 1.0 {
        return 1.0;
    }
    let rest = 1.0 - parameter(fraction);
    1.0 - rest * rest * rest
}

/// The progress's slope (per unit of time fraction) at `fraction`.
pub(crate) fn progress_slope(fraction: f64) -> f64 {
    if !(0.0..1.0).contains(&fraction) {
        return 0.0;
    }
    let t = parameter(fraction);
    let dx = bezier_slope(t, EASE_X1, EASE_X2);
    if dx.abs() < 1e-12 {
        return 0.0;
    }
    3.0 * (1.0 - t) * (1.0 - t) / dx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn millis(duration: Duration) -> f64 {
        duration.as_secs_f64() * 1000.0
    }

    #[test]
    fn a_glide_lasts_as_long_as_chromiums_for_its_distance() {
        for (pixels, expected) in [
            (0.0, 200.0),
            (33.0, 200.0),
            (100.0, 200.0),
            (120.0, 200.0),
            (200.0, (12.0 - 80.0 / 60.0) * 1000.0 / 60.0),
            (300.0, 150.0),
            (480.0, 100.0),
            (2400.0, 100.0),
            (-300.0, 150.0),
        ] {
            let measured = millis(glide_duration(pixels, 1.0));
            assert!(
                (measured - expected).abs() < 1e-6,
                "{pixels} px: {measured} ms, not {expected} ms"
            );
        }
        assert!((millis(glide_duration(100.0, 0.5)) - 100.0).abs() < 1e-6);
        assert!((millis(glide_duration(100.0, 2.0)) - 400.0).abs() < 1e-6);
        assert_eq!(glide_duration(100.0, 0.0), Duration::ZERO);
        assert!((millis(glide_duration(f64::NAN, 1.0)) - 200.0).abs() < 1e-6);
    }

    #[test]
    fn the_curve_leaves_at_speed_and_eases_out_to_rest() {
        // It leaves at 1/0.42 times the average speed, where Chromium's ease-in-out leaves
        // at rest; half the time covers seven eighths of the way.
        assert!((progress_slope(0.0) - START_SLOPE).abs() < 1e-6);
        assert!((START_SLOPE - 1.0 / 0.42).abs() < 1e-12);
        assert!((progress(0.5) - 0.875).abs() < 1e-6);
        // A notch's first 60 Hz frame of 200 ms moves a sixth of it, not a hundredth.
        let first = progress(1.0 / 12.0);
        assert!(first > 0.16 && first < 0.2, "{first}");
        // It ends at rest, slowly, without overshooting or turning back.
        assert!(progress_slope(0.999) < 0.01);
        let mut previous = 0.0;
        let mut slope = f64::INFINITY;
        for step in 1..=100 {
            let fraction = f64::from(step) / 100.0;
            let value = progress(fraction);
            assert!(value >= previous && value <= 1.0, "monotone at {step}");
            let next = progress_slope(fraction);
            assert!(next <= slope + 1e-9, "the speed only falls ({step})");
            previous = value;
            slope = next;
        }
        assert_eq!(previous, 1.0);
        assert_eq!(progress(-1.0), 0.0);
        assert_eq!(progress_slope(1.0), 0.0);
    }

    #[test]
    fn the_scale_is_bounded_and_defaults_to_chromiums_timing() {
        // The scale is this thread's: other tests keep the default.
        assert_eq!(glide_scale(), 1.0);
        set_glide_scale(0.0);
        assert_eq!(glide_scale(), 0.0);
        set_glide_scale(100.0);
        assert_eq!(glide_scale(), MAX_SCALE);
        set_glide_scale(-1.0);
        assert_eq!(glide_scale(), 0.0);
        set_glide_scale(f32::NAN);
        assert_eq!(glide_scale(), 1.0);
        set_glide_scale(1.5);
        assert_eq!(std::thread::spawn(glide_scale).join().unwrap(), 1.0);
        set_glide_scale(1.0);
    }
}
