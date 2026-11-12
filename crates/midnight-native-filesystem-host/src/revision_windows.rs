//! Windows v1 identity: volume, file ID, length, last-write time and change time.
pub(crate) fn matches(expected: &[u8], current: &[u8]) -> bool {
    super::matches_payload(
        expected,
        current,
        b"native.windows.read.v1",
        b"windows-file-v1",
        48,
    )
}

#[cfg(test)]
#[path = "revision_windows_tests.rs"]
mod tests;

/// Compare a validated Windows Find result with the independently captured native file.
/// This wire format carries identity, length, last-write and change time. Full Find
/// validation checks creation time and attributes before and after capture as well.
/// No I/O; unknown formats, foreign platforms and malformed revisions fail closed.
pub fn matches_find_entry(expected: &midnight_core::FindEntry, current: &[u8]) -> bool {
    let Some(payload) = current
        .strip_prefix(b"native.windows.read.v1")
        .or_else(|| current.strip_prefix(b"windows-file-v1"))
    else {
        return false;
    };
    expected.kind == midnight_core::EntryKind::File
        && payload.len() == 48
        && expected.revision.0[5] == 0
        && super::word(payload, 0) == Some(expected.identity.device)
        && payload.get(8..24) == Some(expected.identity.object.as_slice())
        && super::word(payload, 24) == Some(expected.revision.0[0])
        && super::word(payload, 32) == Some(expected.revision.0[1])
        && super::word(payload, 40) == Some(expected.revision.0[4])
}
