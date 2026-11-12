//! Strict compatibility for persisted native file identities, without filesystem access.
#[cfg(any(target_os = "linux", test))]
#[path = "revision_linux.rs"]
mod linux;
#[cfg(any(windows, test))]
#[path = "revision_windows.rs"]
mod windows;

#[cfg(target_os = "linux")]
pub(crate) use linux::matches;
#[cfg(target_os = "linux")]
pub use linux::matches_find_entry;
#[cfg(windows)]
pub(crate) use windows::matches;
#[cfg(windows)]
pub use windows::matches_find_entry;
#[cfg(not(any(windows, target_os = "linux")))]
pub(crate) fn matches(_expected: &[u8], _current: &[u8]) -> bool {
    false
}

#[cfg(any(windows, target_os = "linux", test))]
fn matches_payload(
    expected: &[u8],
    current: &[u8],
    current_prefix: &[u8],
    legacy_prefix: &[u8],
    payload_bytes: usize,
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
    current.len() == payload_bytes && expected == current
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn matches_find_entry(_expected: &midnight_core::FindEntry, _current: &[u8]) -> bool {
    false
}

fn word(payload: &[u8], offset: usize) -> Option<u64> {
    let bytes = payload.get(offset..offset.checked_add(8)?)?;
    Some(u64::from_le_bytes(bytes.try_into().ok()?))
}
