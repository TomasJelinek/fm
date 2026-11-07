use std::{
    ffi::OsString,
    io,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Component, Path, PathBuf, Prefix},
};

/// Add native extended syntax without following any directory link before trust validation.
pub(super) fn installation_path(path: &Path) -> io::Result<PathBuf> {
    let absolute = std::path::absolute(path)?;
    let mut wide: Vec<u16> = absolute
        .as_os_str()
        .encode_wide()
        .map(|unit| {
            if unit == b'/' as u16 {
                b'\\' as u16
            } else {
                unit
            }
        })
        .collect();
    match absolute.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::VerbatimDisk(_) | Prefix::VerbatimUNC(_, _) => {}
            Prefix::Disk(_) => {
                wide.splice(..0, "\\\\?\\".encode_utf16());
            }
            Prefix::UNC(_, _) => {
                wide.splice(..2, "\\\\?\\UNC\\".encode_utf16());
            }
            _ => return Err(io::Error::other("Unsupported installation namespace")),
        },
        _ => return Err(io::Error::other("Installation path must be absolute")),
    }
    Ok(PathBuf::from(OsString::from_wide(&wide)))
}
