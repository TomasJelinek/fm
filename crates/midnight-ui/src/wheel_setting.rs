//! The system's wheel setting for panel notches (`midnight_ui_input::notch_pixels_for`),
//! as the viewer reads it. Windows: "lines to scroll at a time"
//! (`SPI_GETWHEELSCROLLLINES`). Other systems have no such setting and keep the
//! shared default.
#[cfg(windows)]
#[allow(unsafe_code)]
#[path = "wheel_setting_windows.rs"]
mod implementation;

use midnight_ui_input::WheelSetting;

/// The wheel setting the panels use. An acceptance override answers first (feature
/// `acceptance-seams`).
pub(crate) fn system_wheel_setting() -> WheelSetting {
    #[cfg(feature = "acceptance-seams")]
    if let Some(setting) = crate::acceptance_seams::overridden().wheel {
        return setting;
    }
    read_wheel_setting()
}

#[cfg(windows)]
use implementation::system_wheel_setting as read_wheel_setting;

#[cfg(not(windows))]
fn read_wheel_setting() -> WheelSetting {
    WheelSetting::default()
}
