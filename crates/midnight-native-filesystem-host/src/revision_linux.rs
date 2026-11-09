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
