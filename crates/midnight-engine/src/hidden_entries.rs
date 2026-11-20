//! Which entries "Show hidden files" leaves out. Native listings, archive listings
//! and inline-tree projections share one platform policy; nothing is hidden while
//! hidden files are shown.
use midnight_core::Entry;

#[cfg(windows)]
#[path = "hidden_entries_windows.rs"]
mod platform;
#[cfg(unix)]
#[path = "hidden_entries_unix.rs"]
mod platform;

/// Whether `entry` is a hidden entry on this platform.
pub(crate) fn is_hidden(entry: &Entry) -> bool {
    platform::is_hidden(entry)
}

/// Remove hidden entries when `hide` is set, keeping the order of the others.
/// Returns how many were removed.
pub(crate) fn retain_visible(entries: &mut Vec<Entry>, hide: bool) -> usize {
    if !hide {
        return 0;
    }
    let before = entries.len();
    entries.retain(|entry| !is_hidden(entry));
    before - entries.len()
}
