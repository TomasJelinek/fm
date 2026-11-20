//! The Copy/Move dialog's link option on this platform: its label, whether a Move
//! dialog shows it, and the policy each choice must send. Expected values only.
#[cfg(target_os = "linux")]
#[path = "link_option_linux.rs"]
mod platform;
#[cfg(windows)]
#[path = "link_option_windows.rs"]
mod platform;

pub use platform::{LABEL, sent, shown};
