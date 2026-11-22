//! The system's wheel setting for viewer notches (`midnight_ui_input::notch_pixels_for`).
//! Windows: "lines to scroll at a time" (`SPI_GETWHEELSCROLLLINES`), read when a viewer
//! window gains the focus, so a change in the system settings applies from then on.
//! Other systems have no such setting and keep the shared default.
#[cfg(windows)]
#[allow(unsafe_code)]
#[path = "wheel_setting_windows.rs"]
mod implementation;
#[cfg(windows)]
pub(super) use implementation::system_wheel_setting;

#[cfg(not(windows))]
pub(super) fn system_wheel_setting() -> midnight_ui_input::WheelSetting {
    midnight_ui_input::WheelSetting::default()
}
