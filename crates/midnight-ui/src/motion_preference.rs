//! Whether the system asks for less motion. Windows: "Animation effects" off
//! (`SPI_GETCLIENTAREAANIMATION`). Other systems: no setting is read yet.
#[cfg(windows)]
#[allow(unsafe_code)]
#[path = "motion_preference_windows.rs"]
mod implementation;

/// Whether the system shows animations. An acceptance override answers first
/// (feature `acceptance-seams`).
pub(crate) fn animations_enabled() -> bool {
    #[cfg(feature = "acceptance-seams")]
    if let Some(reduce_motion) = crate::acceptance_seams::overridden().reduce_motion {
        return !reduce_motion;
    }
    system_animations_enabled()
}

#[cfg(windows)]
use implementation::animations_enabled as system_animations_enabled;

#[cfg(not(windows))]
fn system_animations_enabled() -> bool {
    true
}
