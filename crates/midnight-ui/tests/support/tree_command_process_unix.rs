use std::path::Path;
pub fn assert_cwd(actual: &[u8], expected: &Path) {
    assert_eq!(actual, super::raw::bytes(expected.as_os_str()));
}
