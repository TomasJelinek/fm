pub(super) fn current() -> Vec<u8> {
    [b"native.linux.read.v1".as_slice(), &[7; 56]].concat()
}
pub(super) fn legacy() -> Vec<u8> {
    [b"linux-file-v1".as_slice(), &[7; 56]].concat()
}
