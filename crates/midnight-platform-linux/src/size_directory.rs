//! Native directory-link resolution; device/inode ancestry prevents recursive cycles.
use midnight_core::{Cancellation, FsError, SizeDirectoryIdentity, SizeDirectoryTarget};
use std::{fs, io, os::unix::fs::MetadataExt, path::Path};

pub(super) fn resolve(
    path: &Path,
    cancel: &Cancellation,
) -> Result<Option<SizeDirectoryTarget>, FsError> {
    let result = (|| {
        if cancel.is_cancelled() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let resolved = fs::canonicalize(path)?;
        let metadata = fs::metadata(&resolved)?;
        if !metadata.is_dir() {
            return Ok(None);
        }
        if cancel.is_cancelled() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        Ok(Some(SizeDirectoryTarget {
            path: resolved,
            identity: SizeDirectoryIdentity {
                device: metadata.dev(),
                object: u128::from(metadata.ino()).to_le_bytes(),
            },
        }))
    })();
    result.map_err(|error| FsError::from_io("resolve directory size target", path, error))
}
