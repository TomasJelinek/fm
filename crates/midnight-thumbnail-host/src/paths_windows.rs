use std::{io, path::PathBuf};

/// Normalize namespace syntax without resolving any reparse point before trust validation.
pub(super) fn installation_path(path: &std::path::Path) -> io::Result<PathBuf> {
    use std::{
        ffi::OsString,
        os::windows::ffi::{OsStrExt, OsStringExt},
        path::{Component, Prefix},
    };
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
            _ => return Err(io::Error::other("Unsupported installation path namespace")),
        },
        _ => return Err(io::Error::other("Installation path must be absolute")),
    }
    Ok(PathBuf::from(OsString::from_wide(&wide)))
}
pub(crate) fn cache_root() -> io::Result<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .map(|path| {
            path.join("Midnight Explorer")
                .join("cache")
                .join("thumbnails")
                .join("v1")
        })
        .ok_or_else(|| io::Error::other("An absolute private LocalAppData directory is required"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        ffi::OsString,
        os::windows::ffi::{OsStrExt, OsStringExt},
    };
    #[test]
    fn installation_prefix_normalization_preserves_native_utf16_without_resolution() {
        let input = PathBuf::from(OsString::from_wide(&[
            67, 58, 92, 0xd800, 92, 109, 101, 46, 101, 120, 101,
        ]));
        let output = installation_path(&input).unwrap();
        let expected: Vec<_> = "\\\\?\\"
            .encode_utf16()
            .chain(input.as_os_str().encode_wide())
            .collect();
        assert_eq!(
            output.as_os_str().encode_wide().collect::<Vec<_>>(),
            expected
        );
        assert_eq!(installation_path(&output).unwrap(), output);
        assert_eq!(
            installation_path(std::path::Path::new(r"\\server\share\me.exe")).unwrap(),
            PathBuf::from(r"\\?\UNC\server\share\me.exe")
        );
    }
}
