//! Options › Appearance › Scrolling glide, applied at once to every scroll surface of the
//! UI thread. The chosen stop's scale (`ScrollGlide::scale`) goes to
//! `midnight_ui_input::set_glide_scale`, which the wheel glides and touchpad momentum of
//! the panels, Slint lists, dialogs, viewer and comparison windows read; all of them run on
//! this thread. The system's request for less motion forces 0, as Off does. The panels'
//! row alignment reads the same scale through the window's `glide-scale`. Scrolling speed
//! (`apply_wheel_speed`) goes the same way to `midnight_ui_input::set_wheel_speed_scale`.
use crate::MainWindow;
use std::cell::Cell;

thread_local! {
    /// The Options choice and the system's less motion, as last applied on this thread.
    static STATE: Cell<(f32, bool)> = const { Cell::new((1.0, false)) };
}

/// The effective scale: the chosen one, or 0 while the system asks for less motion.
fn effective(chosen: f32, reduce: bool) -> f32 {
    if reduce { 0.0 } else { chosen }
}

fn publish(window: &MainWindow, chosen: f32, reduce: bool) {
    STATE.with(|state| state.set((chosen, reduce)));
    let scale = effective(chosen, reduce);
    midnight_ui_input::set_glide_scale(scale);
    window.set_glide_scale(scale);
}

/// The presented Options value (staged while Options is open; Cancel restores it).
pub(crate) fn apply(window: &MainWindow, glide: midnight_core::ScrollGlide) {
    window.set_scroll_glide_index(glide.index());
    let (_, reduce) = STATE.with(Cell::get);
    publish(window, glide.scale(), reduce);
}

/// Options › Appearance › Scrolling speed: the presented share of the system's notch
/// distance, which `midnight_ui_input::notch_pixels_for` applies to every wheel notch of
/// this thread's panels, lists, viewers and comparisons.
pub(crate) fn apply_wheel_speed(window: &MainWindow, speed: midnight_core::WheelSpeed) {
    window.set_wheel_speed_index(speed.index());
    midnight_ui_input::set_wheel_speed_scale(speed.scale());
}

/// The system's request for less motion, as the panels read it (first native event and
/// each focus gain).
pub(crate) fn system_reduces_motion(window: &MainWindow, reduce: bool) {
    let (chosen, _) = STATE.with(Cell::get);
    publish(window, chosen, reduce);
}

#[cfg(test)]
mod tests {
    use super::effective;

    #[test]
    fn less_motion_from_the_system_or_off_turns_gliding_off() {
        assert_eq!(effective(1.0, false), 1.0);
        assert_eq!(effective(4.0, false), 4.0);
        assert_eq!(effective(0.0, false), 0.0);
        assert_eq!(effective(1.0, true), 0.0);
        assert_eq!(effective(4.0, true), 0.0);
    }
}
