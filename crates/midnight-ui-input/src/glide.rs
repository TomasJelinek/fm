//! Placeholder of the scroll-speed assignment's glide knob (same signatures and
//! semantics), so that this export builds before that change lands. `WheelMotion`
//! and `KineticScroll` read it there; at merge their file replaces this one.
use std::cell::Cell;

thread_local! {
    static GLIDE_SCALE: Cell<f32> = const { Cell::new(1.0) };
}

/// The glide scale for this thread's scroll surfaces, 0 to 8; non-finite is 1.
pub fn set_glide_scale(scale: f32) {
    let scale = if scale.is_finite() {
        scale.clamp(0.0, 8.0)
    } else {
        1.0
    };
    GLIDE_SCALE.with(|cell| cell.set(scale));
}

/// The glide scale that new wheel glides and momentum use (1 unless set).
pub fn glide_scale() -> f32 {
    GLIDE_SCALE.with(Cell::get)
}
