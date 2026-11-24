//! Native sources, staging handles and explicitly recoverable root capture.
//! Ordinary Copy sources remain read-only. All blocking work belongs on backend workers.
use crate::mutation::{native, validate_name, validate_path};
use crate::reparse::Native;
use midnight_core::{
    Cancellation, CopyLinkPolicy, EntryKind, EntryMetadata, FsError, MutationItem,
    TransferDirectory, TransferEntry, TransferIdentity, TransferRevisionPolicy, TransferService,
    TransferSource,
};
use std::{fs::File, io, os::windows::fs::MetadataExt, path::Path, sync::Arc};
use windows_sys::Win32::Storage::FileSystem::*;

#[path = "transfer_destination.rs"]
mod destination;
#[path = "transfer_link_destination.rs"]
mod link_destination;
#[path = "transfer_metadata.rs"]
mod metadata;
#[path = "transfer_move.rs"]
mod move_source;
#[path = "transfer_namespace.rs"]
mod namespace;
#[path = "transfer_preserve.rs"]
mod preserve;
#[path = "transfer_source.rs"]
mod source;
#[path = "transfer_source_links.rs"]
mod source_links;
#[path = "transfer_validation.rs"]
mod validation;

impl TransferService for crate::WindowsFileSystem {
    fn native_drop_device(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Option<u64>, FsError> {
        use std::{fs::OpenOptions, os::windows::fs::OpenOptionsExt};
        checked(cancel, path)?;
        validate_path(path, true)?;
        // Advisory metadata only. Operation preparation independently pins and validates
        // its authority; this handle requests neither file contents nor deletion rights.
        let file = OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .map_err(|e| error(path, e))?;
        let attributes = file
            .metadata()
            .map_err(|e| error(path, e))?
            .file_attributes();
        let link = Native::of(&file, attributes)
            .map_err(|e| error(path, e))?
            .link();
        // Advisory: the volume alone, also on filesystems that Copy/Move then refuse. A
        // share is a volume of its own, as for the link facts: a drag from a drive to its
        // own share (`\\localhost\D$`, which reports the drive's serial number) proposes
        // Copy, as a Move there is refused.
        let device = if link {
            None
        } else {
            let volume = native::local_identity(&file)
                .map_err(|e| error(path, e))?
                .volume;
            Some(if crate::storage_kind::network(path) {
                link_destination::share_volume(&file, volume)
            } else {
                volume
            })
        };
        checked(cancel, path)?;
        Ok(device)
    }
    fn prepare_checksum_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        use std::path::{Component, PathBuf};
        use windows_sys::Wdk::Storage::FileSystem::{FILE_DIRECTORY_FILE, FILE_OPEN};
        checked(cancel, path)?;
        validate_path(path, true)?;
        let mut root = PathBuf::new();
        let mut file = None;
        for component in path.components() {
            checked(cancel, path)?;
            match component {
                Component::Prefix(_) => root.push(component.as_os_str()),
                Component::RootDir => {
                    root.push(component.as_os_str());
                    file = Some(native::open_parent(&root).map_err(|e| error(path, e))?);
                }
                Component::Normal(name) => {
                    let parent = file.as_ref().ok_or_else(|| {
                        invalid(path, "checksum directory requires a native root")
                    })?;
                    let child = native::open_relative(
                        parent,
                        name,
                        FILE_READ_ATTRIBUTES | FILE_TRAVERSE | SYNCHRONIZE,
                        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                        FILE_OPEN,
                        FILE_DIRECTORY_FILE,
                    )
                    .map_err(|e| error(path, e))?;
                    if Native::query(&child).map_err(|e| error(path, e))?.link() {
                        return Err(unsupported(
                            path,
                            "checksum directory contains a link or junction",
                        ));
                    }
                    file = Some(child);
                }
                _ => return Err(invalid(path, "checksum directory contains a dot component")),
            }
        }
        let file =
            file.ok_or_else(|| invalid(path, "checksum directory requires a native root"))?;
        destination::Directory::from_file(file, path.to_owned())
            .map(|directory| Arc::new(directory) as Arc<dyn TransferDirectory>)
    }
    fn link_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<midnight_core::TransferLinkDestination, FsError> {
        link_destination::link_destination(path, cancel)
    }
    fn prepare_move_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.prepare_transfer_destination(path, cancel)
    }
    fn prepare_move_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<midnight_core::PreparedTransferMove, FsError> {
        move_source::prepare(path, TransferRevisionPolicy::Exact, cancel)
    }
    fn prepare_move_source_with_links(
        &self,
        path: &Path,
        links: CopyLinkPolicy,
        cancel: &Cancellation,
    ) -> Result<midnight_core::PreparedTransferMoveEntry, FsError> {
        self.prepare_move_source_with_policy(path, links, TransferRevisionPolicy::Exact, cancel)
    }
    fn prepare_move_source_with_policy(
        &self,
        path: &Path,
        links: CopyLinkPolicy,
        revisions: TransferRevisionPolicy,
        cancel: &Cancellation,
    ) -> Result<midnight_core::PreparedTransferMoveEntry, FsError> {
        match links {
            CopyLinkPolicy::Reject => move_source::prepare(path, revisions, cancel)
                .map(midnight_core::PreparedTransferMoveEntry::Ordinary),
            // Preserve moves each link itself (a rename on one volume, recreated and then
            // removed across volumes); FollowLinks copies what it points to and then
            // removes only the link.
            CopyLinkPolicy::FollowDirectoryContents
            | CopyLinkPolicy::EmptyDirectories
            | CopyLinkPolicy::Preserve
            | CopyLinkPolicy::FollowLinks => {
                move_source::prepare_with_links(path, links, revisions, cancel)
            }
        }
    }
    fn prepare_transfer_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.prepare_transfer_source_with_links(path, CopyLinkPolicy::Reject, cancel)
    }
    fn prepare_transfer_source_with_links(
        &self,
        path: &Path,
        links: CopyLinkPolicy,
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
        // Prepared before the job is admitted: no guard on the item until it is used.
        source::root::prepare(parent, parent_path.to_owned(), name, links, cancel)
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
    classified(file, path).map(|(entry, _)| entry)
}
/// Only name-surrogate reparse points are links. Cloud placeholders, deduplicated and
/// WOF-compressed entries are ordinary files/directories whose content is read through
/// their filter (see `reparse::reopen_content`).
fn classified(file: &File, path: &Path) -> Result<(TransferEntry, Native), FsError> {
    classified_with_metadata(file, path).map(|(entry, native, _)| (entry, native))
}
fn classified_with_metadata(
    file: &File,
    path: &Path,
) -> Result<(TransferEntry, Native, std::fs::Metadata), FsError> {
    let metadata = file.metadata().map_err(|e| error(path, e))?;
    let native = Native::of(file, metadata.file_attributes()).map_err(|e| error(path, e))?;
    let directory = native.directory();
    let entry = TransferEntry {
        identity: identity(file, path)?,
        item: MutationItem {
            path: path.to_owned(),
            kind: native.kind(),
            // Raw time only: NLS formatting is presentation, and the engine formats
            // just the entries a prompt displays, never every enumerated entry.
            metadata: EntryMetadata {
                size: (!directory).then_some(metadata.len()),
                modified: metadata.modified().ok(),
                native_attributes: Some(native.attributes),
                ..EntryMetadata::default()
            },
        },
    };
    Ok((entry, native, metadata))
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
