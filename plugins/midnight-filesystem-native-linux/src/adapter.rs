use crate::{content::Reader, system};
use midnight_filesystem_plugin_api::*;
use midnight_filesystem_plugin_sdk::{
    CallContext,
    native::{NativeAdapter, NativeEntry, NativeInfo, NativeRename},
};
use std::{
    ffi::{OsStr, OsString},
    fs::{self, File},
    io::Read,
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::MetadataExt,
    },
    path::{Path, PathBuf},
    sync::Arc,
};
pub struct Linux;
impl NativeAdapter for Linux {
    type Directory = system::Directory;
    type Reader = Reader;
    fn decode(address: &Address) -> Result<PathBuf, Failure> {
        if address.encoding != AddressEncoding::UnixBytes {
            return Err(Failure::new(
                ErrorCode::Unsupported,
                "Linux native byte address required.",
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
        let file = system::open_path(path, libc::O_PATH | libc::O_NOFOLLOW, context)?;
        let mut info = info(&file.metadata().map_err(Failure::from_io)?);
        if info.kind == Kind::Symlink {
            // Match listing hints when the SDK revalidates a directory token,
            // while retaining metadata from the original no-follow descriptor.
            let target = system::open_path(path, libc::O_PATH | libc::O_DIRECTORY, context);
            system::cancel(context)?;
            if target.is_ok() {
                info.kind = Kind::DirectoryLink;
            }
        }
        Ok(info)
    }
    fn list(path: &Path, context: &CallContext<'_>) -> Result<system::Directory, Failure> {
        system::Directory::open(path, context)
    }
    fn next(
        directory: &mut system::Directory,
        context: &CallContext<'_>,
    ) -> Result<Option<NativeEntry>, Failure> {
        directory.next(context)
    }
    fn open_reader(path: &Path, context: &CallContext<'_>) -> Result<Reader, Failure> {
        Reader::open(path, context)
    }
    fn prepare_rename(
        path: &Path,
        context: &CallContext<'_>,
    ) -> Result<Arc<dyn NativeRename>, Failure> {
        crate::rename::prepare(path, context)
    }
    fn volumes(context: &CallContext<'_>) -> Result<Vec<Volume>, Failure> {
        let file = File::open("/proc/self/mountinfo").map_err(Failure::from_io)?;
        let mut input = Vec::new();
        file.take(1024 * 1024 + 1)
            .read_to_end(&mut input)
            .map_err(Failure::from_io)?;
        if input.len() > 1024 * 1024 {
            return Err(Failure::new(
                ErrorCode::ResourceLimit,
                "Mount information budget exceeded.",
            ));
        }
        let mut roots = crate::mounts::paths(&input);
        if !roots.iter().any(|root| root == b"/") {
            roots.insert(0, b"/".to_vec());
        }
        if roots.len() > 4096 {
            return Err(Failure::new(
                ErrorCode::ResourceLimit,
                "Volume count budget exceeded.",
            ));
        }
        let mut volumes = Vec::new();
        for bytes in roots {
            system::cancel(context)?;
            let id = format!(
                "mount:{}",
                bytes
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            );
            let root = PathBuf::from(OsString::from_vec(bytes));
            volumes.push(Volume {
                id,
                display: root.to_string_lossy().into_owned(),
                root: Self::encode(&root),
            });
        }
        Ok(volumes)
    }
    fn storage_kind(path: &Path, context: &CallContext<'_>) -> Result<StorageKind, Failure> {
        Ok(crate::storage_kind::detect(path, &|| {
            context.is_cancelled()
        }))
    }
    fn space_info(path: &Path, context: &CallContext<'_>) -> Result<(u64, u64), Failure> {
        system::space_info(path, context)
    }
    fn volume_label(path: &Path, context: &CallContext<'_>) -> Result<Option<NativeText>, Failure> {
        crate::volume_details::label(path, context)
    }
    fn volume_information(
        path: &Path,
        context: &CallContext<'_>,
    ) -> Result<VolumeInformation, Failure> {
        crate::volume_information::query(path, context)
    }
}
pub fn info(metadata: &fs::Metadata) -> NativeInfo {
    let kind = metadata.file_type();
    let kind = if kind.is_symlink() {
        Kind::Symlink
    } else if kind.is_dir() {
        Kind::Directory
    } else if kind.is_file() {
        Kind::File
    } else {
        Kind::Other
    };
    let nanos = u32::try_from(metadata.mtime_nsec())
        .ok()
        .filter(|nanos| *nanos < 1_000_000_000);
    NativeInfo {
        kind,
        metadata: Metadata {
            size: (kind == Kind::File).then_some(metadata.len()),
            modified: nanos.map(|nanos| Timestamp {
                seconds: metadata.mtime(),
                nanos,
            }),
            unix_mode: Some(metadata.mode()),
            native_attributes: None,
            compression: Compression::Unknown,
        },
    }
}
