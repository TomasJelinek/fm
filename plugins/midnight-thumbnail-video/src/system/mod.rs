//! Platform decoders, each in its own helper executable without FFmpeg: Media Foundation on
//! Windows; the distribution's FFmpeg, then GStreamer, on Linux. They run in the same
//! restricted worker sandbox as the bundled helpers and read only the host-brokered capture.
pub mod pcm;

#[cfg(windows)]
pub mod media_foundation;
#[cfg(all(windows, feature = "playback-worker"))]
pub use media_foundation::playback_main;
#[cfg(windows)]
pub use media_foundation::thumbnail_main;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(all(target_os = "linux", feature = "playback-worker"))]
pub use linux::playback_main;
#[cfg(target_os = "linux")]
pub use linux::thumbnail_main;

#[cfg(not(any(windows, target_os = "linux")))]
pub fn thumbnail_main() -> std::io::Result<()> {
    Err(unavailable())
}

#[cfg(all(not(any(windows, target_os = "linux")), feature = "playback-worker"))]
pub fn playback_main() -> std::io::Result<()> {
    Err(unavailable())
}

#[cfg(not(any(windows, target_os = "linux")))]
fn unavailable() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "No platform video decoder is built for this system",
    )
}
