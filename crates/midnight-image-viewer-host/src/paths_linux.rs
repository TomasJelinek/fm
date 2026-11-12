use std::{
    io,
    path::{Path, PathBuf},
};

pub(super) fn installation_path(path: &Path) -> io::Result<PathBuf> {
    std::path::absolute(path)
}
