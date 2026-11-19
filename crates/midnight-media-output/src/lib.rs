//! Native audio output owned on the backend playback thread.
#![deny(unsafe_code, unsafe_op_in_unsafe_fn)]

use midnight_media_host::AudioSink;
use std::io;

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows;

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
mod linux;

/// Open the default output on the current thread. Keep the sink on this thread
/// for playback, cancellation and destruction.
pub fn open_default() -> io::Result<Box<dyn AudioSink>> {
    #[cfg(windows)]
    return windows::open_default();

    #[cfg(target_os = "linux")]
    return linux::open_default();

    #[cfg(not(any(windows, target_os = "linux")))]
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "native audio output is unavailable on this platform",
    ))
}
