//! Literal native file addresses; display text and DOS parsing cannot retarget a source.
use std::{
    ffi::OsString,
    io,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Component, Path, PathBuf, Prefix},
};

pub(super) fn literal(path: &Path) -> io::Result<PathBuf> {
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "An absolute drive or UNC file path without dot steps is required",
        )
    };
    if !path.is_absolute() {
        return Err(invalid());
    }
    let units: Vec<_> = path.as_os_str().encode_wide().collect();
    if units.contains(&0)
        || units
            .split(|unit| *unit == 47 || *unit == 92)
            .any(|part| part == [46] || part == [46, 46])
    {
        return Err(invalid());
    }
    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return Err(invalid());
    };
    let mut result: Vec<_> = match prefix.kind() {
        Prefix::VerbatimDisk(_) | Prefix::VerbatimUNC(_, _) => return Ok(path.to_owned()),
        Prefix::Disk(_) => r"\\?\".encode_utf16().collect(),
        Prefix::UNC(_, _) => r"\\?\UNC\".encode_utf16().collect(),
        _ => return Err(invalid()),
    };
    let skip = usize::from(matches!(prefix.kind(), Prefix::UNC(_, _))) * 2;
    result.extend(
        units
            .into_iter()
            .skip(skip)
            .map(|unit| if unit == 47 { 92 } else { unit }),
    );
    Ok(PathBuf::from(OsString::from_wide(&result)))
}
