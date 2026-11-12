use midnight_filesystem_plugin_api::{Address, AddressEncoding, NameEncoding};
use std::{os::windows::ffi::OsStrExt, path::Path};
pub(super) fn address(path: &Path) -> Address {
    Address {
        encoding: AddressEncoding::WindowsUtf16Le,
        bytes: path
            .as_os_str()
            .encode_wide()
            .flat_map(u16::to_le_bytes)
            .collect(),
    }
}
pub(super) fn name(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}
pub(super) fn encoding() -> NameEncoding {
    NameEncoding::WindowsUtf16Le
}
