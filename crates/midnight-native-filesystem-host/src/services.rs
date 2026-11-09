use crate::{NativeProvider, client::validate_node, discovery::invalid, paths};
use midnight_core::{
    Cancellation, CompressionStatus, DirectoryService, Entry, EntryId, EntryKind, EntryMetadata,
    FsError, SpaceInfo, StorageService, VolumeService,
};
use midnight_filesystem_plugin_api::{
    ErrorCode, Failure, Kind, Metadata, Node, Request, Response, abi,
};
use std::{
    io,
    path::Path,
    time::{Duration, UNIX_EPOCH},
};

pub(crate) fn entry(node: Node) -> Result<Entry, Failure> {
    let path = paths::decode(&node.address)?;
    let name = paths::name(node.name_encoding, &node.raw_name)?;
    if path.file_name() != Some(name.as_os_str()) {
        return Err(invalid(
            "Filesystem entry name differs from its native address",
        ));
    }
    let kind = match node.kind {
        Kind::Directory => EntryKind::Directory,
        Kind::DirectoryLink => EntryKind::DirectoryLink,
        Kind::File => EntryKind::File,
        Kind::Symlink => EntryKind::Symlink,
        Kind::Other => EntryKind::Other,
    };
    let mut entry = Entry::new(EntryId(node.id), path, kind);
    entry.metadata = metadata(node.metadata)?;
    Ok(entry)
}

fn metadata(metadata: Metadata) -> Result<EntryMetadata, Failure> {
    let modified = metadata
        .modified
        .map(|time| {
            if time.nanos >= 1_000_000_000 {
                return Err(invalid("Invalid native timestamp"));
            }
            let seconds = Duration::from_secs(time.seconds.unsigned_abs());
            let seconds = if time.seconds >= 0 {
                UNIX_EPOCH.checked_add(seconds)
            } else {
                UNIX_EPOCH.checked_sub(seconds)
            };
            seconds
                .and_then(|time_value| {
                    time_value.checked_add(Duration::from_nanos(u64::from(time.nanos)))
                })
                .ok_or_else(|| invalid("Native timestamp cannot be represented on this platform"))
        })
        .transpose()?;
    Ok(EntryMetadata {
        size: metadata.size,
        modified,
        localized_modified: None,
        compression: match metadata.compression {
            midnight_filesystem_plugin_api::Compression::Unknown => CompressionStatus::Unknown,
            midnight_filesystem_plugin_api::Compression::Uncompressed => {
                CompressionStatus::Uncompressed
            }
            midnight_filesystem_plugin_api::Compression::Compressed => {
                CompressionStatus::Compressed
            }
        },
        native_attributes: metadata.native_attributes,
        unix_mode: metadata.unix_mode,
    })
}

impl DirectoryService for NativeProvider {
    fn read_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        NativeProvider::read_directory(self, path, &|| cancel.is_cancelled(), emit)
            .map_err(|failure| fs_error("list directory", path, failure))
    }
    fn listing_metadata_complete(&self, entry: &Entry) -> bool {
        entry.metadata.modified.is_some()
            && (entry.metadata.native_attributes.is_some() || entry.metadata.unix_mode.is_some())
    }
    fn entry_metadata(&self, path: &Path) -> Result<EntryMetadata, FsError> {
        self.metadata(path)
            .map_err(|failure| fs_error("read metadata", path, failure))
    }
}
impl NativeProvider {
    fn metadata(&self, path: &Path) -> Result<EntryMetadata, Failure> {
        self.require(abi::CAP_STAT)?;
        let (_lease, mount, node) = self.resolve(path, &|| false)?;
        let Response::Node(current) = self.exchange(
            Request::Stat {
                mount,
                node: node.id,
            },
            &|| false,
        )?
        else {
            return Err(invalid("Missing metadata response"));
        };
        validate_node(&current)?;
        if current.id != node.id || current.address != node.address {
            return Err(invalid("Metadata node changed its identity"));
        }
        metadata(current.metadata)
    }
    fn volumes(&self) -> Result<Vec<midnight_core::Volume>, Failure> {
        self.require(abi::CAP_VOLUMES)?;
        let Response::Volumes(volumes) = self.exchange(Request::Volumes, &|| false)? else {
            return Err(invalid("Missing volumes response"));
        };
        let mut ids = std::collections::HashSet::new();
        volumes
            .into_iter()
            .map(|volume| {
                if volume.id.is_empty() || !ids.insert(volume.id.clone()) {
                    return Err(invalid("Invalid or repeated native volume identity"));
                }
                Ok(midnight_core::Volume {
                    id: volume.id,
                    label: volume.display,
                    root: paths::decode(&volume.root)?,
                })
            })
            .collect()
    }
    fn storage(&self, path: &Path) -> Result<midnight_core::StorageKind, Failure> {
        self.require(abi::CAP_STORAGE)?;
        let Response::StorageKind(kind) = self.exchange(
            Request::StorageKind {
                address: paths::encode(path)?,
            },
            &|| false,
        )?
        else {
            return Err(invalid("Missing storage kind response"));
        };
        Ok(crate::volumes::storage_kind(kind))
    }
    fn space(&self, path: &Path) -> Result<SpaceInfo, Failure> {
        self.require(abi::CAP_STORAGE)?;
        let Response::SpaceInfo { available, total } = self.exchange(
            Request::SpaceInfo {
                address: paths::encode(path)?,
            },
            &|| false,
        )?
        else {
            return Err(invalid("Missing space information response"));
        };
        if available > total {
            return Err(invalid("Available bytes exceed filesystem size"));
        }
        Ok(SpaceInfo { available, total })
    }
}
impl VolumeService for NativeProvider {
    fn volume_label(&self, path: &Path) -> Result<Option<std::ffi::OsString>, FsError> {
        self.label(path)
            .map_err(|failure| fs_error("query volume label", path, failure))
    }
    fn list_volumes(&self) -> Result<Vec<midnight_core::Volume>, FsError> {
        self.volumes()
            .map_err(|failure| fs_error("list volumes", Path::new(""), failure))
    }
}
impl StorageService for NativeProvider {
    fn volume_information(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<midnight_core::VolumeInformation, FsError> {
        self.information(path, &|| cancel.is_cancelled())
            .map_err(|failure| fs_error("query volume information", path, failure))
    }
    fn storage_kind(&self, path: &Path) -> Result<midnight_core::StorageKind, FsError> {
        self.storage(path)
            .map_err(|failure| fs_error("query storage kind", path, failure))
    }
    fn space_info(&self, path: &Path) -> Result<SpaceInfo, FsError> {
        self.space(path)
            .map_err(|failure| fs_error("query filesystem space", path, failure))
    }
}
pub(crate) fn fs_error(operation: &'static str, path: &Path, failure: Failure) -> FsError {
    let kind = match failure.code {
        ErrorCode::Unsupported => io::ErrorKind::Unsupported,
        ErrorCode::NotFound => io::ErrorKind::NotFound,
        ErrorCode::PermissionDenied => io::ErrorKind::PermissionDenied,
        ErrorCode::Invalid => io::ErrorKind::InvalidData,
        ErrorCode::Cancelled => io::ErrorKind::Interrupted,
        ErrorCode::Busy => io::ErrorKind::WouldBlock,
        ErrorCode::ResourceLimit => io::ErrorKind::OutOfMemory,
        _ => io::ErrorKind::Other,
    };
    FsError::from_io(operation, path, io::Error::new(kind, failure))
}
