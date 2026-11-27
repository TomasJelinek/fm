//! File identities and reparse classification that every Windows file system answers.
//!
//! NTFS and ReFS give every file a 128-bit ID (`FileIdInfo`) and a reparse tag
//! (`FileAttributeTagInfo`). FAT32, exFAT, CD and DVD file systems (CDFS, UDF), the
//! WebDAV redirector and SMB servers before SMB 3 implement neither information class
//! and answer with one of the errors of [`unsupported_class`]. There the 64-bit file
//! index of `GetFileInformationByHandle` stands in for the ID, and the attributes of
//! `FileBasicInfo` for the tag (such file systems have no reparse points). The host
//! (`midnight-plugin-host` and `midnight-platform-windows`) applies the same rules, so a
//! revision this plugin reports and an identity the host's Find records agree; see
//! "File systems" in `docs/developer/content-reads.md`.
use std::{fs::File, io, os::windows::io::AsRawHandle};
use windows_sys::Win32::{
    Foundation::{
        ERROR_CALL_NOT_IMPLEMENTED, ERROR_INVALID_FUNCTION, ERROR_INVALID_LEVEL,
        ERROR_INVALID_PARAMETER, ERROR_NOT_SUPPORTED,
    },
    Storage::FileSystem::*,
};

/// Whether a query failed because the file system or server does not implement the
/// information class, rather than because the handle or the access failed: FAT32 and
/// exFAT answer `ERROR_INVALID_PARAMETER` (measured for `FileIdInfo`), other drivers,
/// redirectors and SMB servers `ERROR_INVALID_FUNCTION`, `ERROR_NOT_SUPPORTED`,
/// `ERROR_INVALID_LEVEL` or `ERROR_CALL_NOT_IMPLEMENTED`. Denials, lost connections and
/// every other error stay errors.
pub(crate) fn unsupported_class(error: &io::Error) -> bool {
    [
        ERROR_INVALID_FUNCTION,
        ERROR_NOT_SUPPORTED,
        ERROR_INVALID_PARAMETER,
        ERROR_CALL_NOT_IMPLEMENTED,
        ERROR_INVALID_LEVEL,
    ]
    .into_iter()
    .any(|code| error.raw_os_error() == Some(code as i32))
}

/// The file's volume serial number and identity: the 128-bit `FileIdInfo` ID, or, where
/// the file system has none (an [`unsupported_class`] error) or reports an all-zero ID
/// (MS-FSCC 2.4.26: a server without 128-bit IDs), the 64-bit file index widened with
/// the 32-bit volume serial number. A volume answers one way for every file, so a
/// revision and its revalidations compare identities of one kind. On FAT the index
/// follows the entry's directory record, and some servers report index 0 for every
/// file: there an identity tells fewer changes apart.
pub(crate) fn file_identity(file: &File) -> io::Result<(u64, [u8; 16])> {
    choose_identity(file_id(file), || file_index(file))
}

fn choose_identity(
    id: io::Result<FILE_ID_INFO>,
    index: impl FnOnce() -> io::Result<BY_HANDLE_FILE_INFORMATION>,
) -> io::Result<(u64, [u8; 16])> {
    match id {
        Ok(id) if id.FileId.Identifier != [0; 16] => {
            return Ok((id.VolumeSerialNumber, id.FileId.Identifier));
        }
        Ok(_) => {}
        Err(error) if unsupported_class(&error) => {}
        Err(error) => return Err(error),
    }
    let information = index()?;
    let index =
        (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow);
    Ok((
        u64::from(information.dwVolumeSerialNumber),
        u128::from(index).to_le_bytes(),
    ))
}

fn file_id(file: &File) -> io::Result<FILE_ID_INFO> {
    let mut id = FILE_ID_INFO::default();
    // SAFETY: file owns the live handle and the output is writable for its exact length.
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&mut id as *mut FILE_ID_INFO).cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(id)
}

fn file_index(file: &File) -> io::Result<BY_HANDLE_FILE_INFORMATION> {
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: file owns the live handle and the output is a valid, writable structure.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(information)
}

/// The file's attributes and reparse tag. Where the file system does not implement
/// `FileAttributeTagInfo` (an [`unsupported_class`] error: FAT32, exFAT and other file
/// systems without reparse points), the attributes come from `FileBasicInfo` with tag 0.
/// Such an entry whose attributes still claim a reparse point keeps tag 0, which names
/// no known reparse kind, so callers refuse it as a link (fail closed).
pub(crate) fn attribute_tag(file: &File) -> io::Result<FILE_ATTRIBUTE_TAG_INFO> {
    choose_attribute_tag(tag(file), || basic(file))
}

fn choose_attribute_tag(
    tag: io::Result<FILE_ATTRIBUTE_TAG_INFO>,
    basic: impl FnOnce() -> io::Result<FILE_BASIC_INFO>,
) -> io::Result<FILE_ATTRIBUTE_TAG_INFO> {
    match tag {
        Err(error) if unsupported_class(&error) => Ok(FILE_ATTRIBUTE_TAG_INFO {
            FileAttributes: basic()?.FileAttributes,
            ReparseTag: 0,
        }),
        result => result,
    }
}

fn tag(file: &File) -> io::Result<FILE_ATTRIBUTE_TAG_INFO> {
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
        return Err(io::Error::last_os_error());
    }
    Ok(info)
}

/// The file's `FileBasicInfo` (times and attributes), which every file system answers.
pub(crate) fn basic(file: &File) -> io::Result<FILE_BASIC_INFO> {
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
        return Err(io::Error::last_os_error());
    }
    Ok(value)
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
