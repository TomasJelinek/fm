#![forbid(unsafe_code)]
//! Selects the native filesystem adapter. Implementations live in dedicated OS crates.

#[cfg(windows)]
pub use fm_platform_windows::{WindowsFileSystem as LocalFileSystem, home_directory};

#[cfg(target_os = "linux")]
pub use fm_platform_linux::{LinuxFileSystem as LocalFileSystem, home_directory};

#[cfg(not(any(windows, target_os = "linux")))]
compile_error!("fm-platform currently supports Windows and Linux targets");
