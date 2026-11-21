//! Video decoders for the restricted thumbnail and playback helpers. The bundled FFmpeg
//! profile (`bundled`) and the platform decoders (`system`) run in separate helper
//! executables; each sees only the bounded host broker stream after worker lockdown.
pub mod media;
pub mod playback_session;
pub mod playback_wire;
pub mod playback_worker;

#[cfg(feature = "bundled")]
mod bundled;
#[cfg(feature = "bundled")]
mod native;
#[cfg(feature = "bundled")]
pub mod playback;
#[cfg(feature = "bundled")]
#[doc(hidden)]
pub mod playback_audio;
#[cfg(feature = "bundled")]
pub use bundled::decode;

#[cfg(feature = "system")]
pub mod system;
