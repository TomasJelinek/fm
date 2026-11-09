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
