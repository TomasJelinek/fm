//! Handle-relative local copies. Anonymous O_TMPFILE staging is required: its
//! cleanup closes an owned inode and cannot unlink a substituted staging pathname.
//! This is not an isolation boundary against root or another process of the same uid.
use midnight_core::{
    Cancellation, EntryKind, EntryMetadata, FsError, MutationItem, TransferDirectory,
    TransferEntry, TransferIdentity, TransferService, TransferSource,
};
use rustix::fs::{CWD, Mode, OFlags, ResolveFlags, openat, openat2};
use std::{
    ffi::OsStr,
    fs::File,
    io,
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::{Path, PathBuf},
    sync::Arc,
};
#[path = "transfer_destination.rs"]
mod destination;
#[cfg(test)]
#[path = "transfer_drag_tests.rs"]
mod drag_tests;
#[path = "transfer_metadata.rs"]
mod metadata;
#[path = "transfer_move.rs"]
mod move_source;
#[path = "transfer_native_location.rs"]
mod native_location;
#[path = "transfer_source.rs"]
mod source;
#[path = "transfer_validation.rs"]
mod validation;
const RESOLVE: ResolveFlags = ResolveFlags::BENEATH
    .union(ResolveFlags::NO_SYMLINKS)
    .union(ResolveFlags::NO_XDEV);
const DIRECTORY: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::CLOEXEC);
impl TransferService for crate::LinuxFileSystem {
    fn native_drop_device(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Option<u64>, FsError> {
        native_location::drop_device(path, cancel)
    }
    fn prepare_checksum_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        checked(cancel, path)?;
        let path = absolute(path)?;
        let file = File::from(
            openat2(
                CWD,
                &path,
                DIRECTORY | OFlags::NOFOLLOW,
                Mode::empty(),
                ResolveFlags::NO_SYMLINKS,
            )
            .map_err(|e| error(&path, e.into()))?,
        );
        Ok(Arc::new(destination::Directory::from_file(file, path)?))
    }
    fn prepare_move_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        // Native rename needs only destination metadata authority. The engine
        // checks proof/replacement qualification before a cross-device fallback.
        self.prepare_transfer_destination(path, cancel)
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
        let path = absolute(path)?;
        let name = path
            .file_name()
            .ok_or_else(|| invalid(&path, "filesystem roots are not copy sources"))?;
        let parent_path = path
            .parent()
            .ok_or_else(|| invalid(&path, "source needs a parent"))?;
        let parent = Arc::new(File::from(
            openat(CWD, parent_path, DIRECTORY, Mode::empty())
                .map_err(|e| error(&path, e.into()))?,
        ));
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
        let path = absolute(path)?;
        let directory = File::from(
            openat(CWD, &path, DIRECTORY, Mode::empty()).map_err(|e| error(&path, e.into()))?,
        );
        Ok(Arc::new(destination::Directory::from_file(
            directory, path,
        )?))
    }
}
fn absolute(path: &Path) -> Result<PathBuf, FsError> {
    if !path.is_absolute()
        || path
            .as_os_str()
            .as_bytes()
            .split(|b| *b == b'/')
            .any(|part| part == b"." || part == b"..")
    {
        return Err(invalid(
            path,
            "an absolute path without dot components is required",
        ));
    }
    Ok(path.to_owned())
}
fn validate_name(name: &OsStr, path: &Path) -> Result<(), FsError> {
    let bytes = name.as_bytes();
    if bytes.is_empty()
        || bytes == b"."
        || bytes == b".."
        || bytes.contains(&0)
        || bytes.contains(&b'/')
    {
        Err(invalid(path, "a single native basename is required"))
    } else {
        Ok(())
    }
}
fn identity(file: &File, path: &Path) -> Result<TransferIdentity, FsError> {
    let metadata = file.metadata().map_err(|e| error(path, e))?;
    let mut object = [0; 16];
    object[..8].copy_from_slice(&metadata.ino().to_le_bytes());
    Ok(TransferIdentity {
        device: metadata.dev(),
        object,
    })
}
fn entry(file: &File, path: &Path) -> Result<TransferEntry, FsError> {
    let metadata = file.metadata().map_err(|e| error(path, e))?;
    let kind = if metadata.is_file() {
        EntryKind::File
    } else if metadata.is_dir() {
        EntryKind::Directory
    } else if metadata.file_type().is_symlink() {
        EntryKind::Symlink
    } else {
        EntryKind::Other
    };
    Ok(TransferEntry {
        identity: identity(file, path)?,
        item: MutationItem {
            path: path.to_owned(),
            kind,
            metadata: EntryMetadata {
                size: metadata.is_file().then_some(metadata.len()),
                modified: metadata.modified().ok(),
                localized_modified: metadata.modified().ok().and_then(crate::locale::modified),
                unix_mode: Some(metadata.mode()),
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
