//! Pin every ancestor without delete sharing; final opens never traverse reparse points.
use super::{Record, valid_name};
use std::{
    fs::{self, File, OpenOptions},
    io,
    os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::{Component, Path, PathBuf},
    ptr,
    sync::atomic::{AtomicU64, Ordering},
};
use windows_sys::{
    Wdk::Storage::FileSystem::{
        FILE_RENAME_INFORMATION, FileRenameInformation, NtSetInformationFile,
    },
    Win32::{
        Foundation::{LocalFree, RtlNtStatusToDosError},
        Security::{Authorization::*, *},
        Storage::FileSystem::*,
        System::{IO::IO_STATUS_BLOCK, SystemServices::ACCESS_ALLOWED_ACE_TYPE},
    },
};

static TEMPORARIES: AtomicU64 = AtomicU64::new(1);

pub(super) struct Root {
    path: PathBuf,
    _pins: Vec<File>,
}
pub(super) struct Temporary {
    pub file: File,
    published: bool,
}
impl Drop for Temporary {
    fn drop(&mut self) {
        if !self.published {
            let _ = delete(&self.file);
        }
    }
}

fn no_reparse(file: &File, directory: bool) -> io::Result<()> {
    let metadata = file.metadata()?;
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || (directory && !metadata.is_dir())
        || (!directory && !metadata.is_file())
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Cache path is not an ordinary file/directory",
        ));
    }
    Ok(())
}

struct Descriptor(*mut std::ffi::c_void);
impl Drop for Descriptor {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}

fn private_directory(path: &Path) -> io::Result<()> {
    let sddl: Vec<u16> = "D:P(A;OICI;FA;;;OW)(A;OICI;FA;;;SY)"
        .encode_utf16()
        .chain([0])
        .collect();
    let mut raw = ptr::null_mut();
    // SAFETY: fixed SDDL and exact native outputs; descriptor retained through CreateDirectory.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut raw,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let descriptor = Descriptor(raw);
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    let raw: Vec<u16> = path.as_os_str().encode_wide().collect();
    let mut path: Vec<u16> = if raw.starts_with(&[92, 92, 63, 92]) {
        raw
    } else if raw.starts_with(&[92, 92]) {
        "\\\\?\\UNC\\"
            .encode_utf16()
            .chain(raw.into_iter().skip(2))
            .collect()
    } else {
        "\\\\?\\".encode_utf16().chain(raw).collect()
    };
    path.push(0);
    // SAFETY: terminated absolute native path; protected ACL exposes no user content before creation.
    if unsafe { CreateDirectoryW(path.as_ptr(), &attributes) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn check_private(file: &File) -> io::Result<()> {
    let mut acl = ptr::null_mut();
    let mut raw = ptr::null_mut();
    // SAFETY: live pinned directory; returned descriptor owns its ACL until freed below.
    let result = unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut acl,
            ptr::null_mut(),
            &mut raw,
        )
    };
    if result != 0 {
        return Err(io::Error::from_raw_os_error(result as i32));
    }
    let _descriptor = Descriptor(raw);
    if acl.is_null() {
        return Err(io::Error::other("Cache needs a private DACL"));
    }
    for index in 0..unsafe { (*acl).AceCount } {
        let mut ace = ptr::null_mut();
        if unsafe { GetAce(acl, index as u32, &mut ace) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let header = unsafe { &*ace.cast::<ACE_HEADER>() };
        if u32::from(header.AceFlags) & INHERIT_ONLY_ACE != 0 {
            continue;
        }
        if u32::from(header.AceType) != ACCESS_ALLOWED_ACE_TYPE
            || usize::from(header.AceSize) < size_of::<ACCESS_ALLOWED_ACE>()
        {
            return Err(io::Error::other("Unsupported cache ACL"));
        }
        let allowed = unsafe { &*ace.cast::<ACCESS_ALLOWED_ACE>() };
        let sid = ptr::addr_of!(allowed.SidStart).cast_mut().cast();
        // Only owner-rights and SYSTEM inherit access. Existing nonprivate directories are refused.
        if unsafe {
            IsWellKnownSid(sid, WinCreatorOwnerRightsSid) == 0
                && IsWellKnownSid(sid, WinLocalSystemSid) == 0
        } {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Cache directory is not private",
            ));
        }
    }
    Ok(())
}

impl Root {
    pub fn open(path: &Path) -> io::Result<Self> {
        if !path.is_absolute() {
            return Err(io::Error::other("Cache root must be absolute"));
        }
        let mut current = PathBuf::new();
        let mut pins = Vec::new();
        for component in path.components() {
            match component {
                Component::Prefix(_) => {
                    current.push(component.as_os_str());
                    continue;
                }
                Component::RootDir | Component::Normal(_) => current.push(component.as_os_str()),
                _ => {
                    return Err(io::Error::other(
                        "Cache root must not contain traversal components",
                    ));
                }
            }
            let open = || {
                OpenOptions::new()
                    .access_mode(FILE_READ_ATTRIBUTES | READ_CONTROL | FILE_TRAVERSE)
                    .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                    .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
                    .open(&current)
            };
            let file = match open() {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    private_directory(&current)?;
                    open()?
                }
                Err(error) => return Err(error),
            };
            no_reparse(&file, true)?;
            pins.push(file);
        }
        check_private(
            pins.last()
                .ok_or_else(|| io::Error::other("Cache root is empty"))?,
        )?;
        Ok(Self {
            path: path.to_owned(),
            _pins: pins,
        })
    }
    pub fn read(&self, name: &str) -> io::Result<File> {
        let file = OpenOptions::new()
            .access_mode(FILE_GENERIC_READ | FILE_WRITE_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(self.path.join(name))?;
        no_reparse(&file, false)?;
        Ok(file)
    }
    pub fn lock_file(&self) -> io::Result<(File, bool)> {
        let open = |create| {
            OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(create)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .open(self.path.join("store.lock"))
        };
        let (file, created) = match open(true) {
            Ok(file) => (file, true),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (open(false)?, false),
            Err(error) => return Err(error),
        };
        no_reparse(&file, false)?;
        Ok((file, created))
    }
    pub fn temporary(&self) -> io::Result<Temporary> {
        for _ in 0..16 {
            let name = format!(
                "owned-{}-{}.tmp",
                std::process::id(),
                TEMPORARIES.fetch_add(1, Ordering::Relaxed)
            );
            match OpenOptions::new()
                .write(true)
                .access_mode(FILE_GENERIC_WRITE | DELETE)
                .share_mode(FILE_SHARE_READ)
                .create_new(true)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .open(self.path.join(name))
            {
                Ok(file) => {
                    return Ok(Temporary {
                        file,
                        published: false,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::other("Cache temporary names exhausted"))
    }
    pub fn publish(&self, mut temporary: Temporary, name: &str) -> io::Result<()> {
        match self.read(name) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let name: Vec<u16> = name.encode_utf16().collect();
        let bytes = size_of::<FILE_RENAME_INFORMATION>() + name.len() * 2;
        let mut buffer = vec![0u64; bytes.div_ceil(8)];
        let information = buffer.as_mut_ptr().cast::<FILE_RENAME_INFORMATION>();
        let mut status = IO_STATUS_BLOCK::default();
        // SAFETY: exclusive owned source handle, aligned counted UTF-16 basename. Null root means
        // rename inside the source's pinned parent; replacement never traverses a target reparse point.
        let result = unsafe {
            (*information).Anonymous.ReplaceIfExists = true;
            (*information).FileNameLength = (name.len() * 2) as u32;
            ptr::copy_nonoverlapping(
                name.as_ptr(),
                (*information).FileName.as_mut_ptr(),
                name.len(),
            );
            NtSetInformationFile(
                temporary.file.as_raw_handle(),
                &mut status,
                information.cast(),
                bytes as u32,
                FileRenameInformation,
            )
        };
        if result < 0 {
            return Err(io::Error::from_raw_os_error(
                unsafe { RtlNtStatusToDosError(result) } as i32,
            ));
        }
        temporary.published = true;
        Ok(())
    }
    pub fn records(&self) -> io::Result<Vec<Record>> {
        let mut records = Vec::new();
        for (index, item) in fs::read_dir(&self.path)?.enumerate() {
            if index >= 100_000 {
                return Err(io::Error::other("Cache directory entry limit exceeded"));
            }
            let item = item?;
            let Some(name) = item
                .file_name()
                .to_str()
                .map(str::to_owned)
                .filter(|name| valid_name(name))
            else {
                continue;
            };
            let Ok(file) = self.read(&name) else {
                continue;
            };
            let metadata = file.metadata()?;
            records.push(Record {
                name,
                bytes: metadata.len(),
                modified: metadata.modified()?,
            });
        }
        Ok(records)
    }
    pub fn remove(&self, name: &str) -> io::Result<()> {
        let file = OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES | DELETE)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(self.path.join(name))?;
        no_reparse(&file, false)?;
        delete(&file)
    }
}

fn delete(file: &File) -> io::Result<()> {
    let information = FILE_DISPOSITION_INFO { DeleteFile: true };
    // SAFETY: ownership refers to the opened object, never a possibly substituted temporary pathname.
    if unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileDispositionInfo,
            (&information as *const FILE_DISPOSITION_INFO).cast(),
            size_of_val(&information) as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
