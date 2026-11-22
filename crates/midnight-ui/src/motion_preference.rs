//! Whether the system asks for less motion. Windows: "Animation effects" off
//! (`SPI_GETCLIENTAREAANIMATION`). Other systems: no setting is read yet.
#[cfg(windows)]
#[allow(unsafe_code)]
#[path = "motion_preference_windows.rs"]
mod implementation;
#[cfg(windows)]
pub(crate) use implementation::animations_enabled;

#[cfg(not(windows))]
pub(crate) fn animations_enabled() -> bool {
    true
}
