//! Chromium's wheel glide: how long a wheel notch's eased motion lasts and the curve it
//! follows (`cc/animation/scroll_offset_animation_curve.cc`, the `kInverseDelta` duration
//! that `ScrollType::kMouseWheel` selects, and its ease-in-out timing function), with one
//! scale for the glide and for touchpad momentum. Pure policy.
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
/// The ease-in-out timing function's first and second control points' x
/// (`kEaseInOutControlPoints`: cubic-bezier(0.42, 0, 0.58, 1)).
const EASE_X1: f64 = 0.42;
const EASE_X2: f64 = 0.58;
/// A new notch during a glide ends no later than reaching the new target at the current
/// speed would, with this allowance for the ease-out (`VelocityBasedDurationBound`).
pub(crate) const VELOCITY_BOUND: f64 = 2.5;

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

/// The glide's progress (0 to 1) at time fraction `fraction` of its duration, on Chromium's
/// ease-in-out curve whose first control point rises to `rise` (0 from rest; a glide that
/// continues a moving one starts at its speed, see [`rise_for`]).
pub(crate) fn progress(fraction: f64, rise: f64) -> f64 {
    if fraction <= 0.0 {
        return 0.0;
    }
    if fraction >= 1.0 {
        return 1.0;
    }
    bezier(parameter(fraction), rise, 1.0)
}

/// The progress's slope (per unit of time fraction) at `fraction`.
pub(crate) fn progress_slope(fraction: f64, rise: f64) -> f64 {
    if !(0.0..1.0).contains(&fraction) {
        return 0.0;
    }
    let t = parameter(fraction);
    let dx = bezier_slope(t, EASE_X1, EASE_X2);
    if dx.abs() < 1e-12 {
        return 0.0;
    }
    bezier_slope(t, rise, 1.0) / dx
}

/// The first control point's rise that starts a glide of `distance` over `duration`
/// at `speed` (pixels a second along the same axis), as Chromium keeps a retargeted
/// glide's speed (`EaseInOutWithInitialSlope`). Bounded to 0..=1, so progress never
/// overshoots or turns back.
pub(crate) fn rise_for(speed: f64, distance: f64, duration: Duration) -> f64 {
    if distance == 0.0 || !speed.is_finite() || speed.signum() != distance.signum() {
        return 0.0;
    }
    let slope = speed * duration.as_secs_f64() / distance;
    (EASE_X1 * slope).clamp(0.0, 1.0)
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
    fn the_curve_is_chromiums_ease_in_out_and_keeps_a_carried_speed() {
        // cubic-bezier(0.42, 0, 0.58, 1) is symmetric: half the time, half the way.
        assert!((progress(0.5, 0.0) - 0.5).abs() < 1e-6);
        assert!((progress(0.25, 0.0) + progress(0.75, 0.0) - 1.0).abs() < 1e-6);
        // It starts and ends at rest, slowly.
        assert!(progress(0.1, 0.0) < 0.03 && progress(0.9, 0.0) > 0.97);
        assert!(progress_slope(0.0, 0.0).abs() < 1e-6);
        let mut previous = 0.0;
        for step in 1..=100 {
            let value = progress(f64::from(step) / 100.0, 0.0);
            assert!(value >= previous, "monotone at {step}");
            previous = value;
        }
        assert_eq!(previous, 1.0);
        // A rise of 0.42 × slope starts the curve at that slope.
        let rise = rise_for(1000.0, 200.0, Duration::from_millis(150));
        assert!((rise - 0.42 * 0.75).abs() < 1e-9);
        assert!((progress_slope(0.0, rise) - 0.75).abs() < 1e-6);
        // A reversed or missing speed starts from rest; a huge one is bounded.
        assert_eq!(rise_for(-1000.0, 200.0, Duration::from_millis(150)), 0.0);
        assert_eq!(rise_for(f64::NAN, 200.0, Duration::from_millis(150)), 0.0);
        assert_eq!(rise_for(1e9, 200.0, Duration::from_millis(150)), 1.0);
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
