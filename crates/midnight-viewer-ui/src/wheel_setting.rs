//! The system's wheel setting for viewer notches (`midnight_ui_input::notch_pixels_for`).
//! Windows: "lines to scroll at a time" (`SPI_GETWHEELSCROLLLINES`), read when a viewer
//! window gains the focus, so a change in the system settings applies from then on.
//! Other systems have no such setting and keep the shared default. The acceptance suite
//! replaces both settings a window reads (`acceptance_seams::override_settings`).
#[cfg(windows)]
#[allow(unsafe_code)]
#[path = "wheel_setting_windows.rs"]
mod implementation;
#[cfg(windows)]
use implementation::system_wheel_setting;

/// The wheel setting a window reads when it gains the focus.
pub(super) fn wheel_setting() -> midnight_ui_input::WheelSetting {
    #[cfg(feature = "acceptance-seams")]
    if let Some(setting) = crate::acceptance_seams::wheel_override() {
        return setting;
    }
    system_wheel_setting()
}

/// Whether the system asks for less motion, as the `host` reads it, read with the wheel
/// setting.
pub(super) fn reduce_motion(host: &dyn Fn() -> bool) -> bool {
    #[cfg(feature = "acceptance-seams")]
    if let Some(reduce) = crate::acceptance_seams::reduce_motion_override() {
        return reduce;
    }
    host()
}

#[cfg(not(windows))]
fn system_wheel_setting() -> midnight_ui_input::WheelSetting {
    midnight_ui_input::WheelSetting::default()
}
