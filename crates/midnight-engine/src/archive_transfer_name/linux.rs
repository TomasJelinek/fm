use midnight_core::FsError;
use std::{ffi::OsString, os::unix::ffi::OsStringExt};

pub(super) fn native_name(raw: &[u8]) -> Result<OsString, FsError> {
    Ok(OsString::from_vec(raw.to_vec()))
}
