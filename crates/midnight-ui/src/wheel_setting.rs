//! The system's wheel setting for panel notches (`midnight_ui_input::notch_pixels_for`),
//! as the viewer reads it. Windows: "lines to scroll at a time"
//! (`SPI_GETWHEELSCROLLLINES`). Other systems have no such setting and keep the
//! shared default.
#[cfg(windows)]
#[allow(unsafe_code)]
#[path = "wheel_setting_windows.rs"]
mod implementation;
#[cfg(windows)]
pub(crate) use implementation::system_wheel_setting;

#[cfg(not(windows))]
pub(crate) fn system_wheel_setting() -> midnight_ui_input::WheelSetting {
    midnight_ui_input::WheelSetting::default()
}
