//! Audited Windows metadata/storage calls; paths remain dynamically sized UTF-16.
use midnight_filesystem_plugin_api::*;
use midnight_filesystem_plugin_sdk::{CallContext, native::NativeAdapter};
use std::{fs::File, os::windows::io::AsRawHandle, path::PathBuf};
use windows_sys::Win32::Storage::FileSystem::*;

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
