#![deny(unsafe_code)]
//! Update checks for Midnight Explorer: GitHub's release list, minisign-signed
//! checksums, resumable downloads, staging and the installation swap with rollback.
//! This crate holds the application's only network code; it runs on its own thread
//! and only when the user asks or enabled the startup check. See
//! `docs/developer/UPDATES.md`.

pub mod apply;
pub mod archive;
pub mod client;
pub mod controller;
pub mod install;
pub mod installer;
pub mod notes;
pub mod release;
pub mod state;
pub mod transport;
pub mod trust;

#[cfg(windows)]
#[path = "http_windows.rs"]
mod http;
#[cfg(target_os = "linux")]
#[path = "http_linux.rs"]
mod http;
#[cfg(any(windows, target_os = "linux"))]
pub use http::SystemTransport;

#[cfg(windows)]
mod installer_windows;

#[cfg(windows)]
#[path = "platform_windows.rs"]
mod platform;
#[cfg(unix)]
#[path = "platform_unix.rs"]
mod platform;

pub use controller::{Offer, Phase, Snapshot, UpdateController};

/// The user's private folder for downloads and the check state: below the local
/// application data on Windows and the cache folder on Linux, never shared with other
/// accounts. `None` without a usable home.
pub fn private_folder() -> Option<std::path::PathBuf> {
    platform::private_folder()
}
