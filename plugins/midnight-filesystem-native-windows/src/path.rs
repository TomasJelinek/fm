//! Preserve names returned by enumeration, including trailing dots/spaces and DOS names.
use midnight_filesystem_plugin_api::{ErrorCode, Failure};
use std::{
    ffi::OsString,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Component, Path, PathBuf, Prefix},
};

pub fn literal(path: &Path) -> Result<PathBuf, Failure> {
    let invalid = || {
        Failure::new(
            ErrorCode::Unsupported,
            "Native path namespace is unsupported.",
        )
    };
    if !path.is_absolute() {
        return Err(invalid());
    }
    let mut raw: Vec<_> = path.as_os_str().encode_wide().collect();
    for unit in &mut raw {
        if *unit == b'/' as u16 {
            *unit = b'\\' as u16;
        }
    }
    // Verbatim paths do not share Win32 dot-component normalization. Refuse
    // ambiguous navigation instead of changing the addressed object.
    if raw
        .split(|unit| *unit == b'\\' as u16)
        .any(|part| part == [46] || part == [46, 46])
    {
        return Err(invalid());
    }
    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return Err(invalid());
    };
    let result = match prefix.kind() {
        Prefix::VerbatimDisk(_) | Prefix::VerbatimUNC(_, _) => return Ok(path.to_owned()),
        Prefix::Disk(_) => "\\\\?\\".encode_utf16().chain(raw).collect::<Vec<_>>(),
        Prefix::UNC(_, _) => "\\\\?\\UNC\\"
            .encode_utf16()
            .chain(raw.into_iter().skip(2))
            .collect(),
        _ => return Err(invalid()),
    };
    Ok(PathBuf::from(OsString::from_wide(&result)))
}
