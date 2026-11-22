use windows_sys::Win32::UI::WindowsAndMessaging::{
    SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW,
};

/// Windows' "Animation effects" (Settings > Accessibility > Visual effects).
/// An unreadable setting counts as on, the system default.
pub(crate) fn animations_enabled() -> bool {
    let mut enabled: windows_sys::core::BOOL = 1;
    // SAFETY: this query writes one BOOL into the live local it is given and
    // retains nothing; fWinIni 0 changes no setting.
    let read = unsafe {
        SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&raw mut enabled).cast(), 0)
    };
    read == 0 || enabled != 0
}
