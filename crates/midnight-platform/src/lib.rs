#![forbid(unsafe_code)]
//! Selects the native filesystem adapter. Implementations live in dedicated OS crates.

#[cfg(windows)]
pub use midnight_platform_windows::{
    NativeClipboard, WindowsFileSystem as LocalFileSystem, home_directory,
};

#[cfg(windows)]
#[path = "network_windows.rs"]
mod network;
#[cfg(target_os = "linux")]
#[path = "network_linux.rs"]
mod network;
pub use network::{locale_service, network_service};

#[cfg(target_os = "linux")]
pub use midnight_platform_linux::{
    LinuxFileSystem as LocalFileSystem, NativeClipboard, home_directory,
};

#[cfg(not(any(windows, target_os = "linux")))]
compile_error!("midnight-platform currently supports Windows and Linux targets");
