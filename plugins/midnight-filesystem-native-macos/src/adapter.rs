use crate::{content::Reader, directory::Directory, system, volumes};
use midnight_filesystem_plugin_api::*;
use midnight_filesystem_plugin_sdk::{
    CallContext,
    native::{NativeAdapter, NativeEntry, NativeInfo},
};
use std::{
    ffi::{OsStr, OsString},
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};

pub struct MacOs;

impl NativeAdapter for MacOs {
    type Directory = Directory;
    type Reader = Reader;

    fn decode(address: &Address) -> Result<PathBuf, Failure> {
        if address.encoding != AddressEncoding::UnixBytes {
            return Err(Failure::new(
                ErrorCode::Unsupported,
                "Darwin native byte address required.",
            ));
        }
        if address.bytes.contains(&0) {
            return Err(Failure::new(ErrorCode::Invalid, "NUL in native address."));
        }
        Ok(PathBuf::from(OsString::from_vec(address.bytes.clone())))
    }
    fn encode(path: &Path) -> Address {
        Address {
            encoding: AddressEncoding::UnixBytes,
            bytes: path.as_os_str().as_bytes().to_vec(),
        }
    }
    fn name(name: &OsStr) -> (NameEncoding, Vec<u8>) {
        (NameEncoding::UnixBytes, name.as_bytes().to_vec())
    }
    fn stat(path: &Path, context: &CallContext<'_>) -> Result<NativeInfo, Failure> {
        Ok(info(&system::stat_path(path, context)?))
    }
    fn list(path: &Path, context: &CallContext<'_>) -> Result<Directory, Failure> {
        Directory::open(path, context)
    }
    fn next(
        directory: &mut Directory,
        context: &CallContext<'_>,
    ) -> Result<Option<NativeEntry>, Failure> {
        directory.next(context)
    }
    fn open_reader(path: &Path, context: &CallContext<'_>) -> Result<Reader, Failure> {
        Reader::open(path, context)
    }
    fn volumes(context: &CallContext<'_>) -> Result<Vec<Volume>, Failure> {
        volumes::list(context)
    }
    fn storage_kind(path: &Path, context: &CallContext<'_>) -> Result<StorageKind, Failure> {
        volumes::storage(path, context)
    }
    fn space_info(path: &Path, context: &CallContext<'_>) -> Result<(u64, u64), Failure> {
        let info = volumes::information(path, context)?;
        match (info.capacity.available, info.capacity.total) {
            (Some(available), Some(total)) => Ok((available, total)),
            _ => Err(Failure::new(
                ErrorCode::Unsupported,
                "Filesystem capacity is unavailable.",
            )),
        }
    }
    fn volume_label(path: &Path, context: &CallContext<'_>) -> Result<Option<NativeText>, Failure> {
        // statfs reports mounted roots/sources, not the filesystem's own volume label.
        // Validate the address without substituting its basename as a false label.
        system::stat_path(path, context)?;
        Ok(None)
    }
    fn volume_information(
        path: &Path,
        context: &CallContext<'_>,
    ) -> Result<VolumeInformation, Failure> {
        volumes::information(path, context)
    }
}

pub(crate) fn info(value: &libc::stat) -> NativeInfo {
    let kind = match value.st_mode & libc::S_IFMT {
        libc::S_IFDIR => Kind::Directory,
        libc::S_IFREG => Kind::File,
        libc::S_IFLNK => Kind::Symlink,
        _ => Kind::Other,
    };
    let nanos = u32::try_from(value.st_mtime_nsec)
        .ok()
        .filter(|value| *value < 1_000_000_000);
    NativeInfo {
        kind,
        metadata: Metadata {
            size: (kind == Kind::File)
                .then(|| u64::try_from(value.st_size).ok())
                .flatten(),
            modified: nanos.map(|nanos| Timestamp {
                seconds: value.st_mtime,
                nanos,
            }),
            unix_mode: Some(u32::from(value.st_mode)),
            native_attributes: None,
            compression: if value.st_flags & libc::UF_COMPRESSED != 0 {
                Compression::Compressed
            } else if kind == Kind::File {
                Compression::Uncompressed
            } else {
                Compression::Unknown
            },
        },
    }
}
