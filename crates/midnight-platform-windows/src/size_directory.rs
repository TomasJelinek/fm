//! Read-only junction resolution and directory identities for cycle-safe size scans.
use midnight_core::{Cancellation, FsError, SizeDirectoryIdentity, SizeDirectoryTarget};
use std::{
    fs::{self, OpenOptions},
    io,
    mem::size_of,
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    path::Path,
};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_INFO, FILE_READ_ATTRIBUTES,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FileIdInfo, GetFileInformationByHandle,
    GetFileInformationByHandleEx,
};

pub(super) fn resolve(
    path: &Path,
    cancel: &Cancellation,
) -> Result<Option<SizeDirectoryTarget>, FsError> {
    let result = (|| {
        if cancel.is_cancelled() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        // canonicalize uses an OS final-path query and opens no contents. In
        // particular it resolves legacy junctions before directory enumeration.
        let resolved = fs::canonicalize(path)?;
        let file = OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(&resolved)?;
        if !file.metadata()?.is_dir() {
            return Ok(None);
        }
        let identity = identity(&file)?;
        if cancel.is_cancelled() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        Ok(Some(SizeDirectoryTarget {
            path: resolved,
            identity,
        }))
    })();
    result.map_err(|error| FsError::from_io("resolve directory size target", path, error))
}

fn unsupported(error: &io::Error) -> bool {
    // ERROR_INVALID_FUNCTION, ERROR_NOT_SUPPORTED and ERROR_INVALID_PARAMETER
    // are returned by filesystems that do not support the extended ID query.
    matches!(error.raw_os_error(), Some(1 | 50 | 87)) || error.kind() == io::ErrorKind::Unsupported
}

fn identity(file: &fs::File) -> io::Result<SizeDirectoryIdentity> {
    let mut information = FILE_ID_INFO::default();
    // SAFETY: file owns the live handle; the writable FILE_ID_INFO buffer
    // remains valid for the synchronous query and has its exact native size.
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&mut information as *mut FILE_ID_INFO).cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    } == 0
    {
        let error = io::Error::last_os_error();
        return if unsupported(&error) {
            legacy_identity(file)
        } else {
            Err(error)
        };
    }
    Ok(SizeDirectoryIdentity {
        device: information.VolumeSerialNumber,
        object: information.FileId.Identifier,
    })
}

fn legacy_identity(file: &fs::File) -> io::Result<SizeDirectoryIdentity> {
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: file retains the handle and information is an exact writable
    // native structure for this synchronous metadata-only query.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) } == 0 {
        let error = io::Error::last_os_error();
        return Err(if unsupported(&error) {
            io::ErrorKind::Unsupported.into()
        } else {
            error
        });
    }
    let index =
        (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow);
    Ok(SizeDirectoryIdentity {
        device: u64::from(information.dwVolumeSerialNumber),
        object: u128::from(index).to_le_bytes(),
    })
}

#[cfg(test)]
#[path = "size_directory_tests.rs"]
mod tests;
