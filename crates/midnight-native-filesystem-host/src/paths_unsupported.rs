use midnight_filesystem_plugin_api::{
    Address, AddressEncoding, ErrorCode, Failure, NameEncoding, NativeText,
};
use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};
pub(crate) const ENCODING: AddressEncoding = AddressEncoding::Opaque;
#[cfg(test)]
pub(crate) const MANIFEST_ENCODING: &str = "unsupported";
pub(crate) fn encode(_: &Path) -> Result<Address, Failure> {
    Err(unsupported())
}
pub(crate) fn decode(_: &Address) -> Result<PathBuf, Failure> {
    Err(unsupported())
}
pub(crate) fn name(_: NameEncoding, _: &[u8]) -> Result<OsString, Failure> {
    Err(unsupported())
}
pub(crate) fn rename_name(_: &OsStr) -> Result<NativeText, Failure> {
    Err(unsupported())
}
fn unsupported() -> Failure {
    Failure::new(
        ErrorCode::Unsupported,
        "Native filesystem address adapter unavailable on this platform",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rename_name_refuses_unsupported_platform() {
        assert_eq!(
            rename_name(std::ffi::OsStr::new("destination.txt"))
                .unwrap_err()
                .code,
            ErrorCode::Unsupported
        );
    }
}
