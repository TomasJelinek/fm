//! Audited Windows metadata/storage calls; paths remain dynamically sized UTF-16.
use midnight_filesystem_plugin_api::*;
use midnight_filesystem_plugin_sdk::{CallContext, native::NativeAdapter};
use std::{fs::File, os::windows::io::AsRawHandle, path::PathBuf};
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::SystemServices::{IO_REPARSE_TAG_CLOUD, IO_REPARSE_TAG_CLOUD_MASK};

/// Inspect the retained handle, including when Windows exposes cloud placeholders.
/// Cloud data is supplied by its filter on this same file; name-surrogate tags
/// (links/mounts) and every other reparse provider remain unsupported.
pub fn require_readable_file(file: &File) -> Result<(), Failure> {
    let mut info = FILE_ATTRIBUTE_TAG_INFO::default();
    // SAFETY: the file owns its live handle; the output has exactly the declared capacity.
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileAttributeTagInfo,
            (&mut info as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
            size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
    } == 0
    {
        return Err(Failure::from_io(std::io::Error::last_os_error()));
    }
    // SAFETY: GetFileType only inspects the retained handle.
    if unsafe { GetFileType(file.as_raw_handle()) } != FILE_TYPE_DISK
        || info.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0
        || (info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
            && !cloud_content_tag(info.ReparseTag))
    {
        return Err(Failure::new(
            ErrorCode::Unsupported,
            "Read requires a regular file or a cloud data placeholder, never a link.",
        ));
    }
    Ok(())
}

fn cloud_content_tag(tag: u32) -> bool {
    // The name-surrogate bit denotes another named entity, not cloud data.
    tag & 0x2000_0000 == 0 && tag & !IO_REPARSE_TAG_CLOUD_MASK == IO_REPARSE_TAG_CLOUD
}

#[cfg(test)]
mod cloud_tests {
    use super::*;
    use windows_sys::Win32::System::SystemServices::{
        IO_REPARSE_TAG_APPEXECLINK, IO_REPARSE_TAG_CLOUD_1, IO_REPARSE_TAG_CLOUD_F,
        IO_REPARSE_TAG_MOUNT_POINT, IO_REPARSE_TAG_PROJFS, IO_REPARSE_TAG_SYMLINK,
    };

    #[test]
    fn cloud_family_is_explicit_and_never_admits_a_name_surrogate() {
        for tag in [
            IO_REPARSE_TAG_CLOUD,
            IO_REPARSE_TAG_CLOUD_1,
            IO_REPARSE_TAG_CLOUD_F,
        ] {
            assert!(cloud_content_tag(tag));
            assert!(!cloud_content_tag(tag | 0x2000_0000));
        }
        for tag in [
            0,
            IO_REPARSE_TAG_MOUNT_POINT,
            IO_REPARSE_TAG_SYMLINK,
            IO_REPARSE_TAG_APPEXECLINK,
            IO_REPARSE_TAG_PROJFS,
            0x8000_1234,
        ] {
            assert!(!cloud_content_tag(tag));
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub length: u64,
    volume: u64,
    id: [u8; 16],
    write: i64,
    change: i64,
}
impl Snapshot {
    pub fn bytes(&self) -> Vec<u8> {
        let mut bytes = b"native.windows.read.v1".to_vec();
        bytes.extend(self.volume.to_le_bytes());
        bytes.extend(self.id);
        bytes.extend(self.length.to_le_bytes());
        bytes.extend(self.write.to_le_bytes());
        bytes.extend(self.change.to_le_bytes());
        bytes
    }
}
fn basic(file: &File) -> Result<FILE_BASIC_INFO, Failure> {
    let mut value = FILE_BASIC_INFO::default();
    // SAFETY: the file owns its live handle and output covers exactly FILE_BASIC_INFO.
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileBasicInfo,
            (&mut value as *mut FILE_BASIC_INFO).cast(),
            size_of::<FILE_BASIC_INFO>() as u32,
        )
    } == 0
    {
        return Err(Failure::from_io(std::io::Error::last_os_error()));
    }
    Ok(value)
}
pub fn snapshot(file: &File) -> Result<Snapshot, Failure> {
    let before = basic(file)?;
    let mut id = FILE_ID_INFO::default();
    let mut standard = FILE_STANDARD_INFO::default();
    // SAFETY: retained live handle and exact writable output structure capacities.
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&mut id as *mut FILE_ID_INFO).cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    } == 0
        || unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileStandardInfo,
                (&mut standard as *mut FILE_STANDARD_INFO).cast(),
                size_of::<FILE_STANDARD_INFO>() as u32,
            )
        } == 0
    {
        return Err(Failure::from_io(std::io::Error::last_os_error()));
    }
    let after = basic(file)?;
    if before.LastWriteTime != after.LastWriteTime || before.ChangeTime != after.ChangeTime {
        return Err(Failure::new(
            ErrorCode::SourceChanged,
            "File changed while capturing its version.",
        ));
    }
    let length = u64::try_from(standard.EndOfFile)
        .map_err(|_| Failure::new(ErrorCode::Invalid, "Invalid native file length."))?;
    Ok(Snapshot {
        length,
        volume: id.VolumeSerialNumber,
        id: id.FileId.Identifier,
        write: after.LastWriteTime,
        change: after.ChangeTime,
    })
}
pub fn volumes(context: &CallContext<'_>) -> Result<Vec<Volume>, Failure> {
    // SAFETY: GetLogicalDrives takes no pointers and returns the native drive bitset.
    let drives = unsafe { GetLogicalDrives() };
    if drives == 0 {
        return Err(Failure::from_io(std::io::Error::last_os_error()));
    }
    let mut result = Vec::new();
    for number in 0..26 {
        if context.is_cancelled() {
            return Err(Failure::new(
                ErrorCode::Cancelled,
                "Volume discovery canceled.",
            ));
        }
        if drives & (1 << number) != 0 {
            let letter = (b'A' + number) as char;
            let path = PathBuf::from(format!("{letter}:\\"));
            result.push(Volume {
                id: format!("drive:{letter}"),
                display: format!("{letter}:"),
                root: crate::adapter::Windows::encode(&path),
            });
        }
    }
    Ok(result)
}
