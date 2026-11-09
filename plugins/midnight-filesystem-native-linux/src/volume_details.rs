//! Bounded kernel metadata reads; raw labels remain native bytes.
use midnight_filesystem_plugin_api::{Failure, NameEncoding, NativeText};
use midnight_filesystem_plugin_sdk::CallContext;
use std::{ffi::OsStr, io::Read, os::unix::ffi::OsStrExt, path::Path};

pub fn read_bounded(path: &Path, maximum: usize, cancelled: &dyn Fn() -> bool) -> Option<Vec<u8>> {
    if cancelled() {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    let mut input = Vec::new();
    file.take(maximum as u64 + 1).read_to_end(&mut input).ok()?;
    (input.len() <= maximum && !cancelled()).then_some(input)
}
pub fn text(value: &OsStr) -> NativeText {
    NativeText {
        encoding: NameEncoding::UnixBytes,
        bytes: value.as_bytes().to_vec(),
    }
}
pub fn label(path: &Path, context: &CallContext<'_>) -> Result<Option<NativeText>, Failure> {
    let file = crate::system::open_path(path, libc::O_PATH, context)?;
    let identity = crate::volume_information::identity(&file)?;
    crate::system::cancel(context)?;
    Ok(crate::volume_information::device_label(
        identity.stx_dev_major,
        identity.stx_dev_minor,
        context,
    ))
}
