//! Metadata-only launch authority and exact handle-normalized Windows path spelling.
use crate::Error;
use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io,
    os::windows::{
        ffi::OsStringExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::{Component, Path, PathBuf, Prefix},
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_NAME_NORMALIZED, FILE_READ_ATTRIBUTES, FILE_SHARE_READ,
    GetFinalPathNameByHandleW, VOLUME_NAME_DOS,
};

pub(super) struct LaunchFile {
    pub(super) executable: PathBuf,
    pub(super) utf16: Vec<u16>,
    pub(super) length: u64,
    pub(super) last_write_time_100ns: i64,
    _file: File,
}

pub(super) fn open(path: &Path) -> Result<LaunchFile, Error> {
    let entered = super::installation_path(path)?;
    super::plain_path(&entered)?;
    let file = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&entered)?;
    super::regular_file(&file.metadata()?)?;
    let utf16 = normalized_path(&file)?;
    let executable = PathBuf::from(OsString::from_wide(&utf16));
    if !executable.is_absolute()
        || !matches!(executable.components().next(), Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::VerbatimDisk(_) | Prefix::VerbatimUNC(_, _)))
    {
        return Err(Error(
            "Native plugin runtime launch path is not a verbatim DOS or UNC path".into(),
        ));
    }
    // Check both spellings after native normalization; neither may hide a reparse point.
    super::plain_path(&entered)?;
    super::plain_path(&executable)?;
    super::regular_file(&fs::symlink_metadata(&executable)?)?;
    let metadata = file.metadata()?;
    super::regular_file(&metadata)?;
    let last_write_time_100ns = i64::try_from(metadata.last_write_time()).map_err(|_| {
        Error("Plugin runtime launch LastWriteTime exceeds the signed range".into())
    })?;
    Ok(LaunchFile {
        executable,
        utf16,
        length: metadata.file_size(),
        last_write_time_100ns,
        _file: file,
    })
}

fn normalized_path(file: &File) -> Result<Vec<u16>, Error> {
    let mut buffer = vec![0_u16; 256];
    loop {
        let capacity = u32::try_from(buffer.len())
            .map_err(|_| Error("Native plugin runtime launch path capacity exceeds u32".into()))?;
        // SAFETY: the live metadata handle and writable UTF-16 buffer remain valid
        // throughout this synchronous call. The returned length is checked before
        // retaining any units, and an insufficient buffer is grown before retrying.
        let length = unsafe {
            GetFinalPathNameByHandleW(
                file.as_raw_handle(),
                buffer.as_mut_ptr(),
                capacity,
                FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
            )
        };
        if length == 0 {
            return Err(io::Error::last_os_error().into());
        }
        if length >= capacity {
            let next = length
                .checked_add(1)
                .and_then(|length| usize::try_from(length).ok())
                .ok_or_else(|| Error("Native plugin runtime launch path length overflow".into()))?;
            buffer
                .try_reserve_exact(next - buffer.len())
                .map_err(|_| Error("Cannot allocate native plugin runtime launch path".into()))?;
            buffer.resize(next, 0);
            continue;
        }
        let length = usize::try_from(length)
            .map_err(|_| Error("Native plugin runtime launch path length exceeds usize".into()))?;
        buffer.truncate(length);
        if buffer.is_empty() || buffer.contains(&0) {
            return Err(Error(
                "Native plugin runtime launch path is empty or contains NUL".into(),
            ));
        }
        return Ok(buffer);
    }
}
