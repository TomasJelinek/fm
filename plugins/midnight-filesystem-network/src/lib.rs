//! Independent network discovery policy and per-platform adapters.
#![deny(unsafe_code, unsafe_op_in_unsafe_fn)]
pub use midnight_filesystem_plugin_api as api;
pub mod adapter;
mod address;
mod catalog;
pub use catalog::NetworkFilesystem;

#[cfg(windows)]
#[path = "entry_windows.rs"]
mod entry;
#[cfg(target_os = "linux")]
#[path = "entry_linux.rs"]
mod entry;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(windows)]
pub mod windows;
