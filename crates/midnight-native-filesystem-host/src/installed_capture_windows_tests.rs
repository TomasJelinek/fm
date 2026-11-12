pub(super) fn current() -> Vec<u8> {
    [b"native.windows.read.v1".as_slice(), &[7; 48]].concat()
}
pub(super) fn legacy() -> Vec<u8> {
    [b"windows-file-v1".as_slice(), &[7; 48]].concat()
}
