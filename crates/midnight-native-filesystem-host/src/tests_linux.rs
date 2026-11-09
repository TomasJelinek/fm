use midnight_filesystem_plugin_api::{Address, AddressEncoding, NameEncoding};
use std::{os::unix::ffi::OsStrExt, path::Path};
pub(super) fn address(path: &Path) -> Address {
    Address {
        encoding: AddressEncoding::UnixBytes,
        bytes: path.as_os_str().as_bytes().to_vec(),
    }
}
pub(super) fn name(text: &str) -> Vec<u8> {
    text.as_bytes().to_vec()
}
pub(super) fn encoding() -> NameEncoding {
    NameEncoding::UnixBytes
}
