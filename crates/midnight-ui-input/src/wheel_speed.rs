//! The application's mouse-wheel speed (Options › Appearance › Scrolling speed): one scale
//! on the system's notch distance that every host reads through
//! [`crate::notch_pixels_for`] when a notch arrives. 1 is the system's distance; one screen
//! per notch, touchpad finger motion and keyboard scrolling do not scale.
//!
//! It belongs to the UI thread, where every scroll surface lives, so each thread (each
//! test among them) starts with the system's distance.
use std::cell::Cell;

thread_local! {
    static SCALE: Cell<f32> = const { Cell::new(1.0) };
}
/// The bounds of the scale: a quarter to four times the system's notch.
const MIN_SCALE: f32 = 0.25;
const MAX_SCALE: f32 = 4.0;

/// This thread's wheel speed scale (1 unless set).
pub fn wheel_speed_scale() -> f32 {
    SCALE.with(Cell::get)
}

/// Set this thread's [`wheel_speed_scale`] for notches from now on (0.25 to 4; not finite
/// or not positive: 1).
pub fn set_wheel_speed_scale(scale: f32) {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale.clamp(MIN_SCALE, MAX_SCALE)
    } else {
        1.0
    };
    SCALE.with(|cell| cell.set(scale));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scale_starts_at_the_systems_notch_and_is_bounded() {
        assert_eq!(wheel_speed_scale(), 1.0);
        set_wheel_speed_scale(2.0);
        assert_eq!(wheel_speed_scale(), 2.0);
        set_wheel_speed_scale(100.0);
        assert_eq!(wheel_speed_scale(), MAX_SCALE);
        set_wheel_speed_scale(0.01);
        assert_eq!(wheel_speed_scale(), MIN_SCALE);
        for invalid in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            set_wheel_speed_scale(invalid);
            assert_eq!(wheel_speed_scale(), 1.0, "{invalid}");
        }
        set_wheel_speed_scale(1.5);
        assert_eq!(std::thread::spawn(wheel_speed_scale).join().unwrap(), 1.0);
        set_wheel_speed_scale(1.0);
    }
}
