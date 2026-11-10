#[cfg(feature = "native-cef")]
mod find;
#[cfg(feature = "native-cef")]
mod native;
#[cfg(feature = "native-cef")]
mod platform;
pub mod policy;
#[cfg(feature = "native-cef")]
mod selection;
pub mod surface;

/// Starts the native Linux process, dispatching CEF subprocesses before IPC.
#[cfg(all(feature = "native-cef", target_os = "linux"))]
pub fn linux_main() -> i32 {
    platform::run()
}
