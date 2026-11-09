use midnight_filesystem_plugin_api::{Address, AddressEncoding, ErrorCode, Failure, NameEncoding};
use std::{
    ffi::OsString,
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
fn unsupported() -> Failure {
    Failure::new(
        ErrorCode::Unsupported,
        "Native filesystem address adapter unavailable on this platform",
    )
}
