//! Windows native boundary: raw UTF-16 paths, handle identity and restricted DLL search.
use crate::{Error, api::abi};
use std::{
    fmt,
    fs::{File, OpenOptions},
    io,
    os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::Path,
};
use windows_sys::Win32::{
    Foundation::{
        ERROR_CALL_NOT_IMPLEMENTED, ERROR_INVALID_FUNCTION, ERROR_INVALID_LEVEL,
        ERROR_INVALID_PARAMETER, ERROR_NOT_SUPPORTED, FreeLibrary, HMODULE,
    },
    Storage::FileSystem::*,
    System::LibraryLoader::*,
};
#[path = "native_windows_paths.rs"]
mod paths;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Snapshot {
    pub length: u64,
    volume: u64,
    id: [u8; 16],
    write: i64,
    change: i64,
}

impl Snapshot {
    /// Compare executable identity under an uninterrupted read-data handle which
    /// denies write/delete sharing. That lease permits ignoring ChangeTime; captured
    /// content uses `matches` under its capture's explicit policy.
    pub(crate) fn matches_leased_executable(&self, after: &Self) -> bool {
        self.same_except_change_time(after)
    }

    /// Whether `current` is still this captured revision under `policy`.
    pub(crate) fn matches(&self, current: &Self, policy: crate::RevisionPolicy) -> bool {
        match policy {
            crate::RevisionPolicy::Exact => self == current,
            crate::RevisionPolicy::AcceptChangeTimeOnly => self.same_except_change_time(current),
        }
    }

    /// Same volume, file ID, length and LastWriteTime; ChangeTime may differ.
    fn same_except_change_time(&self, other: &Self) -> bool {
        self.volume == other.volume
            && self.id == other.id
            && self.length == other.length
            && self.write == other.write
    }

    pub(crate) fn version_bytes(&self) -> Vec<u8> {
        let mut bytes = b"windows-file-v1".to_vec();
        bytes.extend(self.volume.to_le_bytes());
        bytes.extend(self.id);
        bytes.extend(self.length.to_le_bytes());
        bytes.extend(self.write.to_le_bytes());
        bytes.extend(self.change.to_le_bytes());
        bytes
    }

    /// Format only the differing values from already captured handle snapshots.
    pub(crate) fn changed_fields(&self, after: &Self) -> String {
        let mut fields = Vec::new();
        if self.length != after.length {
            fields.push(format!(
                "length(before={}, after={})",
                self.length, after.length,
            ));
        }
        if self.volume != after.volume {
            fields.push(format!(
                "volume(before={}, after={})",
                self.volume, after.volume,
            ));
        }
        if self.id != after.id {
            fields.push(format!(
                "file_id(before={}, after={})",
                DiagnosticFileId(&self.id),
                DiagnosticFileId(&after.id),
            ));
        }
        if self.write != after.write {
            fields.push(format!(
                "LastWriteTime(before={}, after={})",
                self.write, after.write,
            ));
        }
        if self.change != after.change {
            fields.push(format!(
                "ChangeTime(before={}, after={})",
                self.change, after.change,
            ));
        }
        fields.join(", ")
    }
}

struct DiagnosticFileId<'a>(&'a [u8; 16]);

impl fmt::Display for DiagnosticFileId<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

pub(crate) fn open_read(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(paths::literal(path)?)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Viewer needs a regular non-reparse file",
        ));
    }
    Ok(file)
}

/// Return the lease handle before any fallible metadata inspection. The caller
/// must adopt its owner immediately, then verify regular-file and inheritance state.
pub(crate) fn open_executable_lease(path: &Path) -> io::Result<File> {
    if path
        .file_name()
        .is_none_or(|name| name.encode_wide().any(|unit| unit == u16::from(b':')))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Trusted executable must name the default file data stream",
        ));
    }
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(paths::literal(path)?)
}

pub(crate) fn snapshot(file: &File) -> io::Result<Snapshot> {
    let mut basic = FILE_BASIC_INFO::default();
    // SAFETY: file owns the live handle and the output is writable for its exact length.
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileBasicInfo,
            (&mut basic as *mut FILE_BASIC_INFO).cast(),
            size_of::<FILE_BASIC_INFO>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let (volume, id) = identity(file)?;
    Ok(Snapshot {
        length: file.metadata()?.len(),
        volume,
        id,
        write: basic.LastWriteTime,
        change: basic.ChangeTime,
    })
}

/// The file's volume and identity: `FileIdInfo`'s 128-bit file ID, or, where the file
/// system has none ([`no_file_id`]) or reports an all-zero ID (MS-FSCC 2.4.26: a server
/// without 128-bit IDs), the 64-bit file index of `GetFileInformationByHandle`, widened.
/// The filesystem plugin and the host's Find follow the same rule, so their identities
/// of one file agree. A volume answers one way for every file, so a capture and its
/// revalidations always compare identities of one kind. FAT file systems also report no
/// change time (always 0) and keep write times in 2-second steps, and some network
/// servers report a zero index for every file, so there a revision detects fewer
/// changes ("File systems" in `docs/developer/content-reads.md`).
fn identity(file: &File) -> io::Result<(u64, [u8; 16])> {
    choose_identity(file_id(file), || {
        let mut information = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: file owns the live handle and the output is a valid, writable structure.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(information)
    })
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

/// [`identity`]'s choice between a `FileIdInfo` answer and the file index, which is
/// asked for only when the 128-bit ID is missing or all zero.
fn choose_identity(
    id: io::Result<FILE_ID_INFO>,
    index: impl FnOnce() -> io::Result<BY_HANDLE_FILE_INFORMATION>,
) -> io::Result<(u64, [u8; 16])> {
    match id {
        Ok(id) if id.FileId.Identifier != [0; 16] => {
            return Ok((id.VolumeSerialNumber, id.FileId.Identifier));
        }
        Ok(_) => {}
        Err(error) if no_file_id(&error) => {}
        Err(error) => return Err(error),
    }
    Ok(index_identity(&index()?))
}

/// Whether `FileIdInfo` failed because the file system or server has no 128-bit file ID,
/// rather than because the handle failed: FAT32 and exFAT answer
/// `ERROR_INVALID_PARAMETER` (measured), and other drivers and SMB servers that lack the
/// information class answer `ERROR_INVALID_FUNCTION`, `ERROR_NOT_SUPPORTED`,
/// `ERROR_INVALID_LEVEL` or `ERROR_CALL_NOT_IMPLEMENTED` (CD and DVD file systems,
/// WebDAV, SMB 2 servers). The same set as `midnight-platform-windows` and the native
/// filesystem plugin ("File systems" in `docs/developer/content-reads.md`).
fn no_file_id(error: &io::Error) -> bool {
    [
        ERROR_INVALID_PARAMETER,
        ERROR_INVALID_FUNCTION,
        ERROR_NOT_SUPPORTED,
        ERROR_INVALID_LEVEL,
        ERROR_CALL_NOT_IMPLEMENTED,
    ]
    .into_iter()
    .any(|code| error.raw_os_error() == Some(code as i32))
}

/// `GetFileInformationByHandle`'s volume serial number and 64-bit file index as a
/// snapshot identity: the index in the low eight bytes, little-endian, the rest zero.
fn index_identity(information: &BY_HANDLE_FILE_INFORMATION) -> (u64, [u8; 16]) {
    let index =
        (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow);
    let mut id = [0; 16];
    id[..8].copy_from_slice(&index.to_le_bytes());
    (u64::from(information.dwVolumeSerialNumber), id)
}

pub(crate) struct Library(HMODULE);
// SAFETY: module handle may be used on any backend thread; Arc ownership prevents unload.
unsafe impl Send for Library {}
unsafe impl Sync for Library {}
impl Library {
    pub(crate) fn symbol(
        &self,
        name: &std::ffi::CStr,
    ) -> Result<std::ptr::NonNull<std::ffi::c_void>, Error> {
        // SAFETY: retained module and borrowed NUL-terminated symbol name.
        let function = unsafe { GetProcAddress(self.0, name.as_ptr().cast()) }
            .ok_or_else(|| Error(format!("Missing plugin export {}", name.to_string_lossy())))?;
        std::ptr::NonNull::new(function as *mut std::ffi::c_void)
            .ok_or_else(|| Error("Null plugin export".into()))
    }
    pub(crate) fn owns_address(&self, address: *const std::ffi::c_void) -> bool {
        let mut module = std::ptr::null_mut();
        // SAFETY: FROM_ADDRESS treats the argument as an address, not a string;
        // UNCHANGED_REFCOUNT does not transfer ownership of the returned module.
        unsafe {
            GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
                    | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                address.cast(),
                &mut module,
            ) != 0
                && module == self.0
        }
    }
    pub(crate) fn open(path: &Path) -> Result<Self, Error> {
        let wide: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        if wide[..wide.len() - 1].contains(&0) {
            return Err(Error("NUL in library path".into()));
        }
        // SAFETY: absolute validated path, terminated UTF-16, no application/CWD/PATH search.
        let handle = unsafe {
            LoadLibraryExW(
                wide.as_ptr(),
                std::ptr::null_mut(),
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        };
        if handle.is_null() {
            return Err(io::Error::last_os_error().into());
        }
        Ok(Self(handle))
    }
    pub(crate) fn entry(&self) -> Result<abi::Entry, Error> {
        // SAFETY: live module and static terminated export name; ABI dictates the signature.
        let entry = unsafe { GetProcAddress(self.0, c"midnight_plugin_entry_v1".as_ptr().cast()) };
        entry
            .map(|function| unsafe {
                std::mem::transmute::<unsafe extern "system" fn() -> isize, abi::Entry>(function)
            })
            .ok_or_else(|| Error("Missing midnight_plugin_entry_v1 export".into()))
    }
}
impl Drop for Library {
    fn drop(&mut self) {
        // SAFETY: unique owned loader reference, all callbacks quiesce before last Arc drop.
        unsafe {
            FreeLibrary(self.0);
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::Snapshot;

    pub(crate) fn snapshot(
        length: u64,
        volume: u64,
        id: [u8; 16],
        write: i64,
        change: i64,
    ) -> Snapshot {
        Snapshot {
            length,
            volume,
            id,
            write,
            change,
        }
    }
}

#[cfg(test)]
mod revision_policy_tests {
    use super::test_support::snapshot;
    use crate::RevisionPolicy::{AcceptChangeTimeOnly, Exact};

    #[test]
    fn only_a_moved_change_time_is_accepted_and_only_when_asked() {
        let captured = snapshot(3, 7, [5; 16], 11, 13);
        for policy in [Exact, AcceptChangeTimeOnly] {
            assert!(captured.matches(&captured.clone(), policy));
        }
        let drifted = snapshot(3, 7, [5; 16], 11, 14);
        assert!(!captured.matches(&drifted, Exact));
        assert!(captured.matches(&drifted, AcceptChangeTimeOnly));
        // Length, volume, file ID and LastWriteTime each still count, with or without
        // a moved ChangeTime.
        for change in [13, 14] {
            for current in [
                snapshot(4, 7, [5; 16], 11, change),
                snapshot(3, 8, [5; 16], 11, change),
                snapshot(3, 7, [6; 16], 11, change),
                snapshot(3, 7, [5; 16], 12, change),
            ] {
                for policy in [Exact, AcceptChangeTimeOnly] {
                    assert!(!captured.matches(&current, policy), "{current:?}");
                }
            }
        }
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;
    use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_NETNAME_DELETED};

    /// FAT32 and exFAT have no 128-bit file ID: the 64-bit file index stands in, its
    /// high and low halves in place and the volume serial widened.
    #[test]
    fn a_file_index_identity_keeps_both_halves_and_the_volume() {
        let information = BY_HANDLE_FILE_INFORMATION {
            dwVolumeSerialNumber: 0x1731_1e74,
            nFileIndexHigh: 0x0102_0304,
            nFileIndexLow: 0x0506_0708,
            ..Default::default()
        };
        let (volume, id) = index_identity(&information);
        assert_eq!(volume, 0x1731_1e74);
        assert_eq!(id[..8], 0x0102_0304_0506_0708_u64.to_le_bytes());
        assert_eq!(id[8..], [0; 8]);
    }

    /// Only answers that mean "no 128-bit file ID here" fall back to the file index; a
    /// failing handle or a denial stays an error.
    #[test]
    fn only_a_missing_file_id_falls_back_to_the_file_index() {
        for code in [
            ERROR_INVALID_PARAMETER,
            ERROR_INVALID_FUNCTION,
            ERROR_NOT_SUPPORTED,
            ERROR_INVALID_LEVEL,
            ERROR_CALL_NOT_IMPLEMENTED,
        ] {
            assert!(
                no_file_id(&io::Error::from_raw_os_error(code as i32)),
                "{code}"
            );
            let identity = choose_identity(Err(io::Error::from_raw_os_error(code as i32)), || {
                Ok(BY_HANDLE_FILE_INFORMATION {
                    dwVolumeSerialNumber: 9,
                    nFileIndexLow: 4,
                    ..Default::default()
                })
            })
            .unwrap();
            assert_eq!(identity.0, 9);
            assert_eq!(identity.1[0], 4);
        }
        for code in [ERROR_ACCESS_DENIED, ERROR_NETNAME_DELETED] {
            let error = io::Error::from_raw_os_error(code as i32);
            assert!(!no_file_id(&error));
            let failure = choose_identity(Err(error), || -> io::Result<_> {
                panic!("only a missing ID falls back")
            })
            .unwrap_err();
            assert_eq!(failure.raw_os_error(), Some(code as i32));
        }
        assert!(!no_file_id(&io::Error::other("no OS code")));
    }

    /// A server that answers `FileIdInfo` with an all-zero ID has no 128-bit IDs: the
    /// file index stands in, as in the filesystem plugin and the host's Find. A real
    /// 128-bit ID is used as reported, without asking for the index.
    #[test]
    fn an_all_zero_file_id_falls_back_to_the_file_index() {
        let index = || {
            Ok(BY_HANDLE_FILE_INFORMATION {
                dwVolumeSerialNumber: 0x1731_1e74,
                nFileIndexLow: 0x55,
                ..Default::default()
            })
        };
        let zero = FILE_ID_INFO {
            VolumeSerialNumber: 0x0123_4567_89ab_cdef,
            FileId: FILE_ID_128 {
                Identifier: [0; 16],
            },
        };
        let (volume, id) = choose_identity(Ok(zero), index).unwrap();
        assert_eq!(volume, 0x1731_1e74);
        assert_eq!(id[0], 0x55);
        let real = FILE_ID_INFO {
            FileId: FILE_ID_128 {
                Identifier: [3; 16],
            },
            ..zero
        };
        let identity = choose_identity(Ok(real), || -> io::Result<_> {
            panic!("a real ID needs no index")
        })
        .unwrap();
        assert_eq!(identity, (0x0123_4567_89ab_cdef, [3; 16]));
    }

    /// A file on a FAT32 or exFAT volume, named by `MIDNIGHT_TEST_FAT_FILE` (opened for
    /// reading only), is captured, read and revalidated; before the file-index fallback
    /// its capture failed with ERROR_INVALID_PARAMETER, so no viewer, thumbnail,
    /// comparison or archive could read it.
    #[test]
    #[ignore = "needs MIDNIGHT_TEST_FAT_FILE on a FAT32 or exFAT volume"]
    fn a_file_on_a_fat_volume_is_captured_read_and_revalidated() {
        // CI runs this crate's ignored native tests; without a FAT file there is nothing
        // to check.
        let Some(path) = std::env::var_os("MIDNIGHT_TEST_FAT_FILE").map(std::path::PathBuf::from)
        else {
            eprintln!("skipped: MIDNIGHT_TEST_FAT_FILE names no file on a FAT volume");
            return;
        };
        let captured = crate::CapturedFile::capture(&path).unwrap();
        assert!(captured.length() > 0);
        let head = captured
            .read_at(0, 64.min(captured.length() as u32))
            .unwrap();
        assert!(!head.is_empty());
        captured.revalidate().unwrap();
        let again = crate::CapturedFile::capture(&path).unwrap();
        assert!(!captured.change_time_alone_moved(&again));
        again.revalidate().unwrap();
    }
}
