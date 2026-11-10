use crate::store::native::Root;
use std::{
    fs::File,
    io,
    os::windows::{ffi::OsStrExt, fs::FileExt, io::FromRawHandle},
    ptr,
    sync::atomic::{AtomicU64, Ordering},
};
use windows_sys::Win32::{
    Foundation::{INVALID_HANDLE_VALUE, LocalFree},
    Security::{
        Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW, SECURITY_ATTRIBUTES,
    },
    Storage::FileSystem::*,
};

static NEXT_FILE: AtomicU64 = AtomicU64::new(1);
#[cfg(test)]
#[path = "spool_windows_tests.rs"]
mod tests;

struct Descriptor(*mut std::ffi::c_void);
impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: descriptor comes from the allocating native SDDL conversion below.
        unsafe {
            LocalFree(self.0);
        }
    }
}

pub(super) fn create(root: &Root) -> io::Result<File> {
    let sddl: Vec<u16> = "D:P(A;;FA;;;OW)(A;;FA;;;SY)"
        .encode_utf16()
        .chain([0])
        .collect();
    let mut raw = ptr::null_mut();
    // SAFETY: fixed terminated SDDL and valid output; its allocation lives through file creation.
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
    for _ in 0..16 {
        let path = root.path().join(format!(
            "spool-{}-{}.tmp",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
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
        // SAFETY: root keeps all ancestors pinned; CREATE_NEW cannot open an existing
        // object or traverse a final reparse point. The explicit DACL is private,
        // handle inheritance and sharing are disabled, and OS close deletes the file.
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                FILE_GENERIC_READ | FILE_GENERIC_WRITE | DELETE,
                0,
                &attributes,
                CREATE_NEW,
                FILE_ATTRIBUTE_TEMPORARY | FILE_FLAG_DELETE_ON_CLOSE | FILE_FLAG_OPEN_REPARSE_POINT,
                ptr::null_mut(),
            )
        };
        if handle != INVALID_HANDLE_VALUE {
            // SAFETY: successful CreateFileW returned a new exclusively owned handle.
            return Ok(unsafe { File::from_raw_handle(handle) });
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::AlreadyExists {
            return Err(error);
        }
    }
    Err(io::Error::other("Private spool temporary names exhausted"))
}

pub(super) fn read_at(file: &File, offset: u64, output: &mut [u8]) -> io::Result<usize> {
    file.seek_read(output, offset)
}
