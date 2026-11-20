//! Compatibility of captured native file revisions, without filesystem access: exact, or
//! (for viewer reads) tolerating a moved change time.
#[cfg(any(target_os = "linux", test))]
#[path = "revision_linux.rs"]
mod linux;
#[cfg(any(windows, test))]
#[path = "revision_windows.rs"]
mod windows;

use midnight_plugin_host::RevisionPolicy;

#[cfg(target_os = "linux")]
pub(crate) use linux::matches;
#[cfg(target_os = "linux")]
pub use linux::matches_find_entry;
#[cfg(windows)]
pub(crate) use windows::matches;
#[cfg(windows)]
pub use windows::matches_find_entry;
#[cfg(not(any(windows, target_os = "linux")))]
pub(crate) fn matches(_expected: &[u8], _current: &[u8], _policy: RevisionPolicy) -> bool {
    false
}

/// Both revisions are this platform's known layout and `current` still matches `expected`
/// under `policy`. The payload ends with the change time, `change_bytes` long, which
/// `RevisionPolicy::AcceptChangeTimeOnly` leaves out of the comparison.
#[cfg(any(windows, target_os = "linux", test))]
fn matches_payload(
    expected: &[u8],
    current: &[u8],
    current_prefix: &[u8],
    legacy_prefix: &[u8],
    payload_bytes: usize,
    change_bytes: usize,
    policy: RevisionPolicy,
) -> bool {
    let Some(current) = current.strip_prefix(current_prefix) else {
        return false;
    };
    let Some(expected) = expected
        .strip_prefix(current_prefix)
        .or_else(|| expected.strip_prefix(legacy_prefix))
    else {
        return false;
    };
    if current.len() != payload_bytes || expected.len() != payload_bytes {
        return false;
    }
    let compared = match policy {
        RevisionPolicy::Exact => payload_bytes,
        RevisionPolicy::AcceptChangeTimeOnly => payload_bytes - change_bytes,
    };
    expected[..compared] == current[..compared]
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn matches_find_entry(_expected: &midnight_core::FindEntry, _current: &[u8]) -> bool {
    false
}

fn word(payload: &[u8], offset: usize) -> Option<u64> {
    let bytes = payload.get(offset..offset.checked_add(8)?)?;
    Some(u64::from_le_bytes(bytes.try_into().ok()?))
}
