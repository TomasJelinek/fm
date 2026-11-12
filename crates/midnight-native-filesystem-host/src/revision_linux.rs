//! Linux v1 identity: device, inode, length and modification/change seconds and nanoseconds.
pub(crate) fn matches(expected: &[u8], current: &[u8]) -> bool {
    super::matches_payload(
        expected,
        current,
        b"native.linux.read.v1",
        b"linux-file-v1",
        56,
    )
}

#[cfg(test)]
#[path = "revision_linux_tests.rs"]
mod tests;

/// Compare a validated Linux Find result with the independently captured native file.
/// This wire format carries identity, length, mtime and ctime. Full Find validation
/// additionally checks native mode before and after capture.
/// No I/O; unknown formats, foreign platforms and malformed revisions fail closed.
pub fn matches_find_entry(expected: &midnight_core::FindEntry, current: &[u8]) -> bool {
    let Some(payload) = current
        .strip_prefix(b"native.linux.read.v1")
        .or_else(|| current.strip_prefix(b"linux-file-v1"))
    else {
        return false;
    };
    expected.kind == midnight_core::EntryKind::File
        && payload.len() == 56
        && expected.identity.object[8..].iter().all(|byte| *byte == 0)
        && super::word(payload, 0) == Some(expected.identity.device)
        && payload.get(8..16) == Some(&expected.identity.object[..8])
        && (0..5)
            .all(|index| super::word(payload, 16 + index * 8) == Some(expected.revision.0[index]))
}
