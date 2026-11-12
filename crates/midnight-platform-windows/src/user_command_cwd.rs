//! Cmd rejects verbatim paths after CreateProcess succeeds, silently changing cwd.
use crate::mutation::native;
use std::{
    ffi::OsString,
    io,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Component, Path, PathBuf, Prefix},
};

pub(super) fn for_cmd(path: &Path) -> io::Result<PathBuf> {
    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return Err(unsupported());
    };
    match prefix.kind() {
        Prefix::Disk(_) => Ok(path.to_owned()),
        Prefix::VerbatimDisk(_) => {
            let units: Vec<_> = path.as_os_str().encode_wide().collect();
            // Only this typed prefix admits removing \\?\; retain every native unit.
            let candidate = PathBuf::from(OsString::from_wide(&units[4..]));
            // Ordinary DOS parsing may normalize dots/spaces or interpret device
            // names differently. Compare two live native owners before accepting.
            let original = native::open_parent(path).map_err(|_| unsupported())?;
            let converted = native::open_parent(&candidate).map_err(|_| unsupported())?;
            let original_id = native::identity(&original).map_err(|_| unsupported())?;
            let converted_id = native::identity(&converted).map_err(|_| unsupported())?;
            if original_id != converted_id {
                return Err(unsupported());
            }
            Ok(candidate)
        }
        _ => Err(unsupported()),
    }
}

fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "Command Prompt requires an equivalent ordinary DOS working directory. Choose PowerShell for UNC, device or verbatim-only names.",
    )
}
