//! Native startup-placement hints; persisted geometry stays outside the UI adapter.

#[cfg(target_os = "linux")]
#[path = "window_position_linux.rs"]
mod platform;
#[cfg(windows)]
#[path = "window_position_windows.rs"]
mod platform;

/// Request a restored physical outer-frame origin before the native window is shown.
///
/// Returns whether a supported native backend submitted the request. Wayland owns
/// absolute placement and returns false. Window size and state are managed separately.
pub fn restore_window_position(window: &slint::Window, position: slint::PhysicalPosition) -> bool {
    platform::restore(window, position)
}
