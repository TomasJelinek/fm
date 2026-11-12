use midnight_core::FsError;
use std::ffi::OsString;

pub(super) fn native_name(raw: &[u8]) -> Result<OsString, FsError> {
    std::str::from_utf8(raw).map(OsString::from).map_err(|_| {
        super::failure("Archive member name cannot be represented losslessly as Windows Unicode.")
    })
}
