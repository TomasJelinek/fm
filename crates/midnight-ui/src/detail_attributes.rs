//! Platform-specific display policy only; native metadata is already supplied by the backend.
#[cfg(windows)]
#[path = "detail_attributes/windows.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "detail_attributes/linux.rs"]
mod platform;

pub(crate) use platform::{explanation, fields};
