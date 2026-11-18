use std::{io, path::PathBuf};

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
