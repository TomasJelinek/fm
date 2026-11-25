#![forbid(unsafe_code)]
//! Selects the native filesystem adapter. Implementations live in dedicated OS crates.

#[cfg(windows)]
pub use midnight_platform_windows::{
    NativeClipboard, RUN_BATCH_ARGUMENT, WindowsFileSystem as LocalFileSystem, home_directory,
    monitor_work_area, run_and_wait, run_captured_batch, set_console_launcher,
};

#[cfg(target_os = "linux")]
#[path = "window_geometry_linux.rs"]
mod window_geometry;
#[cfg(target_os = "linux")]
pub use window_geometry::monitor_work_area;

#[cfg(windows)]
#[path = "network_windows.rs"]
mod network;
#[cfg(target_os = "linux")]
#[path = "network_linux.rs"]
mod network;
pub use network::{locale_service, network_service};

#[cfg(target_os = "linux")]
pub use midnight_platform_linux::{
    LinuxFileSystem as LocalFileSystem, NativeClipboard, home_directory, raise_open_file_limit,
    run_and_wait, set_console_launcher, with_original_open_file_limit,
};

/// Windows has no soft limit on open handles to raise: the Linux function raises the
/// process's soft open-file limit at start. Returns the soft limit in force, `None` here.
#[cfg(windows)]
pub fn raise_open_file_limit() -> std::io::Result<Option<u64>> {
    Ok(None)
}

/// Windows raises no limit, so a started program has nothing to get back: the Linux
/// function gives it the open-file limit the process started with.
#[cfg(windows)]
pub fn with_original_open_file_limit(_command: &mut std::process::Command) {}

#[cfg(not(any(windows, target_os = "linux")))]
compile_error!("midnight-platform currently supports Windows and Linux targets");
