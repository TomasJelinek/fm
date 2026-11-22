use midnight_ui_input::WheelSetting;
use windows_sys::Win32::UI::WindowsAndMessaging::{SPI_GETWHEELSCROLLLINES, SystemParametersInfoW};

/// `WinUser.h`'s `WHEEL_PAGESCROLL` (`UINT_MAX`): one screen per notch. windows-sys
/// declares it only with its `Win32_UI_Controls` feature.
const WHEEL_PAGESCROLL: u32 = u32::MAX;

/// Windows' "Choose how many lines to scroll each time" (Settings > Bluetooth & devices >
/// Mouse). An unreadable setting counts as the system default.
pub(crate) fn system_wheel_setting() -> WheelSetting {
    let mut lines: u32 = 0;
    // SAFETY: this query writes one UINT into the live local it is given and retains
    // nothing; fWinIni 0 changes no setting.
    let read =
        unsafe { SystemParametersInfoW(SPI_GETWHEELSCROLLLINES, 0, (&raw mut lines).cast(), 0) };
    match (read != 0, lines) {
        (false, _) => WheelSetting::default(),
        (true, WHEEL_PAGESCROLL) => WheelSetting::Page,
        (true, lines) => WheelSetting::Lines(lines),
    }
}
