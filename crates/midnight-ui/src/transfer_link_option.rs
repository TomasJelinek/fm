//! Platform-specific link option of the Copy/Move dialog: its words and the link
//! policy the backend applies. Display and request policy only; the native
//! providers decide what each policy does.
#[cfg(target_os = "linux")]
#[path = "transfer_link_option/linux.rs"]
mod platform;
#[cfg(windows)]
#[path = "transfer_link_option/windows.rs"]
mod platform;

pub(crate) use platform::{HINT, LABEL, policy, visible};
