use crate::{content::Reader, system};
use midnight_filesystem_plugin_api::*;
use midnight_filesystem_plugin_sdk::{
    CallContext,
    native::{NativeAdapter, NativeEntry, NativeInfo},
};
use std::{
    ffi::{OsStr, OsString},
    fs::{self, ReadDir},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::{FileTypeExt, MetadataExt},
    },
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub struct Windows;
impl NativeAdapter for Windows {
    type Directory = ReadDir;
    type Reader = Reader;
    fn decode(address: &Address) -> Result<PathBuf, Failure> {
        if address.encoding != AddressEncoding::WindowsUtf16Le {
            return Err(Failure::new(
                ErrorCode::Unsupported,
                "Windows native address encoding required.",
            ));
        }
        let (units, remainder) = address.bytes.as_chunks::<2>();
        if !remainder.is_empty() || units.contains(&[0, 0]) {
            return Err(Failure::new(
                ErrorCode::Invalid,
                "Invalid UTF-16 native address.",
            ));
        }
        let units: Vec<_> = units.iter().map(|unit| u16::from_le_bytes(*unit)).collect();
        Ok(PathBuf::from(OsString::from_wide(&units)))
    }
    fn encode(path: &Path) -> Address {
        Address {
            encoding: AddressEncoding::WindowsUtf16Le,
            bytes: wide(path.as_os_str()),
        }
    }
    fn name(name: &OsStr) -> (NameEncoding, Vec<u8>) {
        (NameEncoding::WindowsUtf16Le, wide(name))
    }
    fn stat(path: &Path, _context: &CallContext<'_>) -> Result<NativeInfo, Failure> {
        let metadata =
            fs::symlink_metadata(crate::path::literal(path)?).map_err(Failure::from_io)?;
        Ok(info(metadata.file_type(), Some(&metadata)))
    }
    fn list(path: &Path, _context: &CallContext<'_>) -> Result<ReadDir, Failure> {
        fs::read_dir(crate::path::literal(path)?).map_err(Failure::from_io)
    }
    fn next(
        directory: &mut ReadDir,
        context: &CallContext<'_>,
    ) -> Result<Option<NativeEntry>, Failure> {
        if context.is_cancelled() {
            return Err(Failure::new(ErrorCode::Cancelled, "Listing canceled."));
        }
        let Some(entry) = directory.next() else {
            return Ok(None);
        };
        let entry = entry.map_err(Failure::from_io)?;
        enumerated(entry).map(Some)
    }
    fn open_reader(path: &Path, _context: &CallContext<'_>) -> Result<Reader, Failure> {
        Reader::open(path)
    }
    fn prepare_rename(
        path: &Path,
        context: &CallContext<'_>,
    ) -> Result<std::sync::Arc<dyn midnight_filesystem_plugin_sdk::native::NativeRename>, Failure>
    {
        crate::rename::prepare(path, context)
    }
    fn volumes(context: &CallContext<'_>) -> Result<Vec<Volume>, Failure> {
        system::volumes(context)
    }
    fn storage_kind(path: &Path, context: &CallContext<'_>) -> Result<StorageKind, Failure> {
        Ok(crate::storage_kind::detect(path, &|| {
            context.is_cancelled()
        }))
    }
    fn space_info(path: &Path, context: &CallContext<'_>) -> Result<(u64, u64), Failure> {
        crate::volume_details::space(path, context)
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
fn enumerated(entry: fs::DirEntry) -> Result<NativeEntry, Failure> {
    let kind = entry.file_type().map_err(Failure::from_io)?;
    // Windows retains FindData in DirEntry; this avoids an extra OS call per row
    // and preserves the enumeration snapshot even if the name disappears.
    let metadata = entry.metadata().ok();
    Ok(NativeEntry {
        name: entry.file_name(),
        info: info(
            metadata.as_ref().map_or(kind, |value| value.file_type()),
            metadata.as_ref(),
        ),
    })
}

fn wide(value: &OsStr) -> Vec<u8> {
    value.encode_wide().flat_map(u16::to_le_bytes).collect()
}
pub(crate) fn info(kind: fs::FileType, metadata: Option<&fs::Metadata>) -> NativeInfo {
    let kind = if kind.is_symlink_dir() {
        Kind::DirectoryLink
    } else if kind.is_symlink() {
        Kind::Symlink
    } else if kind.is_dir() {
        Kind::Directory
    } else if kind.is_file() {
        Kind::File
    } else {
        Kind::Other
    };
    let metadata = metadata
        .map(|metadata| {
            let attributes = metadata.file_attributes();
            Metadata {
                size: (kind == Kind::File).then_some(metadata.len()),
                modified: metadata.modified().ok().and_then(timestamp),
                native_attributes: Some(attributes),
                unix_mode: None,
                compression: if attributes
                    & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_COMPRESSED
                    != 0
                {
                    Compression::Compressed
                } else {
                    Compression::Uncompressed
                },
            }
        })
        .unwrap_or_default();
    NativeInfo { kind, metadata }
}
fn timestamp(time: SystemTime) -> Option<Timestamp> {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => Some(Timestamp {
            seconds: i64::try_from(duration.as_secs()).ok()?,
            nanos: duration.subsec_nanos(),
        }),
        Err(error) => {
            let duration = error.duration();
            let seconds = i64::try_from(duration.as_secs()).ok()?;
            let nanos = duration.subsec_nanos();
            if nanos == 0 {
                Some(Timestamp {
                    seconds: seconds.checked_neg()?,
                    nanos: 0,
                })
            } else {
                Some(Timestamp {
                    seconds: seconds.checked_neg()?.checked_sub(1)?,
                    nanos: 1_000_000_000 - nanos,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enumeration_retains_cached_metadata_after_the_native_name_disappears() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../_build/goal/native-volume-parity/listing-fixtures");
        fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let directory = root.join(format!("cached-{}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                fs::remove_dir(&self.0).unwrap();
            }
        }
        let _cleanup = Cleanup(directory.clone());
        let path = directory.join("cached.bin");
        fs::write(&path, b"data").unwrap();
        let entry = fs::read_dir(&directory).unwrap().next().unwrap().unwrap();
        fs::remove_file(path).unwrap();
        let entry = enumerated(entry).unwrap();
        assert_eq!(entry.info.kind, Kind::File);
        assert_eq!(entry.info.metadata.size, Some(4));
        assert!(entry.info.metadata.native_attributes.is_some());
    }
}
