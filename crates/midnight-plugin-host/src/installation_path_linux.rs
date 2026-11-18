use std::{
    io,
    path::{Path, PathBuf},
};

/// The absolute spelling of an installation path that trust validation inspects. A relative
/// path is resolved against the current directory by `std::path::absolute`, which is lexical:
/// it follows no link and keeps `..` components.
pub fn installation_path(path: &Path) -> io::Result<PathBuf> {
    std::path::absolute(path)
}
