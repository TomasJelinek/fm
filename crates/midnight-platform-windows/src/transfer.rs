//! Native sources, staging handles and explicitly recoverable root capture.
//! Ordinary Copy sources remain read-only. All blocking work belongs on backend workers.
use crate::mutation::{native, validate_name, validate_path};
use midnight_core::{
    Cancellation, EntryKind, EntryMetadata, FsError, MutationItem, TransferDirectory,
    TransferEntry, TransferIdentity, TransferService, TransferSource,
};
use std::{fs::File, io, os::windows::fs::MetadataExt, path::Path, sync::Arc};
use windows_sys::Win32::Storage::FileSystem::*;

#[path = "transfer_destination.rs"]
mod destination;
#[path = "transfer_move.rs"]
mod move_source;
#[path = "transfer_source.rs"]
mod source;
#[path = "transfer_validation.rs"]
mod validation;

impl TransferService for crate::WindowsFileSystem {
    fn prepare_move_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        let directory = self.prepare_transfer_destination(path, cancel)?;
        if !directory.capabilities().recoverable_replacement {
            return Err(unsupported(
                path,
                "Verified Move requires a qualified local destination. Choose Copy to retain the source.",
            ));
        }
        Ok(directory)
    }
    fn prepare_move_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<midnight_core::PreparedTransferMove, FsError> {
        move_source::prepare(path, cancel)
    }
    fn prepare_transfer_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        checked(cancel, path)?;
        validate_path(path, false)?;
        let parent_path = path
            .parent()
            .ok_or_else(|| invalid(path, "source needs a parent"))?;
        let name = path
            .file_name()
            .ok_or_else(|| invalid(path, "source needs a basename"))?;
        let parent = Arc::new(native::open_parent(parent_path).map_err(|e| error(path, e))?);
        Ok(Box::new(source::Source::open(
            parent,
            parent_path.to_owned(),
            name,
            cancel,
        )?))
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        checked(cancel, path)?;
        validate_path(path, true)?;
        let file = native::open_parent(path).map_err(|e| error(path, e))?;
        destination::Directory::from_file(file, path.to_owned())
            .map(|d| Arc::new(d) as Arc<dyn TransferDirectory>)
    }
}
fn identity(file: &File, path: &Path) -> Result<TransferIdentity, FsError> {
    let id = native::identity(file).map_err(|e| error(path, e))?;
    Ok(TransferIdentity {
        device: id.volume,
        object: id.id,
    })
}
fn entry(file: &File, path: &Path) -> Result<TransferEntry, FsError> {
    let metadata = file.metadata().map_err(|e| error(path, e))?;
    let attributes = metadata.file_attributes();
    let directory = attributes & FILE_ATTRIBUTE_DIRECTORY != 0;
    let link = attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    Ok(TransferEntry {
        identity: identity(file, path)?,
        item: MutationItem {
            path: path.to_owned(),
            kind: match (directory, link) {
                (true, true) => EntryKind::DirectoryLink,
                (false, true) => EntryKind::Symlink,
                (true, false) => EntryKind::Directory,
                (false, false) => EntryKind::File,
            },
            metadata: EntryMetadata {
                size: (!directory).then_some(metadata.len()),
                modified: metadata.modified().ok(),
                localized_modified: metadata.modified().ok().and_then(crate::locale::modified),
                native_attributes: Some(attributes),
                ..EntryMetadata::default()
            },
        },
    })
}
fn checked(cancel: &Cancellation, path: &Path) -> Result<(), FsError> {
    if cancel.is_cancelled() {
        Err(error(path, io::ErrorKind::Interrupted.into()))
    } else {
        Ok(())
    }
}
fn error(path: &Path, error: io::Error) -> FsError {
    FsError::from_io("copy file", path, error)
}
fn invalid(path: &Path, message: &str) -> FsError {
    error(path, io::Error::new(io::ErrorKind::InvalidInput, message))
}
fn unsupported(path: &Path, message: &str) -> FsError {
    error(path, io::Error::new(io::ErrorKind::Unsupported, message))
}
