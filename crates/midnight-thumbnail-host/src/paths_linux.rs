use std::{io, path::PathBuf};

pub(super) fn installation_path(path: &std::path::Path) -> io::Result<PathBuf> {
    std::path::absolute(path)
}
pub(crate) fn cache_root() -> io::Result<PathBuf> {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|path| PathBuf::from(path).join(".cache")))
        .filter(|path| path.is_absolute())
        .map(|path| path.join("midnight-explorer").join("thumbnails").join("v1"))
        .ok_or_else(|| io::Error::other("An absolute private XDG cache directory is required"))
}
