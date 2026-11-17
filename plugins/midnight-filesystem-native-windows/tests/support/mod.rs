#![allow(dead_code)]
use midnight_filesystem_plugin_api::{abi::*, *};
use std::{
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

pub struct Client {
    pub api: Api,
    pub instance: u64,
}
impl Client {
    pub fn new() -> Self {
        let mut api = Api::default();
        // SAFETY: the exported entry borrows the complete aligned caller-owned table.
        assert_eq!(
            unsafe {
                midnight_filesystem_native_windows::midnight_filesystem_entry_v1(
                    &mut api,
                    size_of::<Api>() as u32,
                )
            },
            OK
        );
        Self::from_api(api)
    }
    pub fn from_api(api: Api) -> Self {
        api.validate(size_of::<Api>() as u32).unwrap();
        let mut instance = 0;
        // SAFETY: live Config and instance output are borrowed until the synchronous return.
        assert_eq!(
            unsafe { api.create.unwrap()(&Config::default(), &mut instance) },
            OK
        );
        Self { api, instance }
    }
    pub fn call(&self, request: Request) -> Response {
        self.cancelled(request, 0)
    }
    pub fn cancelled(&self, request: Request, cancel: u64) -> Response {
        unsafe extern "C" fn callback(value: u64) -> u32 {
            u32::from(value != 0)
        }
        self.invoke(request, cancel, callback)
    }
    /// Cancellation follows a live flag that the test may set during the call.
    pub fn flagged(&self, request: Request, cancelled: &AtomicBool) -> Response {
        unsafe extern "C" fn callback(context: u64) -> u32 {
            // SAFETY: `flagged` passes the address of its borrowed flag; the
            // synchronous call uses this context only until it returns.
            let flag = unsafe { &*(context as usize as *const AtomicBool) };
            u32::from(flag.load(Ordering::Acquire))
        }
        self.invoke(
            request,
            cancelled as *const AtomicBool as usize as u64,
            callback,
        )
    }
    fn invoke(
        &self,
        request: Request,
        context: u64,
        callback: unsafe extern "C" fn(u64) -> u32,
    ) -> Response {
        let bytes = encode_request(&request, DEFAULT_REQUEST_BYTES as usize).unwrap();
        let request = Call {
            struct_size: size_of::<Call>() as u32,
            operation: request.operation() as u32,
            payload: Slice {
                data: bytes.as_ptr(),
                len: bytes.len() as u64,
            },
            cancel: Cancellation {
                context,
                cancelled: Some(callback),
            },
        };
        let mut output = Buffer::default();
        // SAFETY: input bytes/callback/output remain mapped throughout the call.
        assert_eq!(
            unsafe { self.api.call.unwrap()(self.instance, &request, &mut output) },
            OK
        );
        struct Release<'a>(&'a Client, u64);
        impl Drop for Release<'_> {
            fn drop(&mut self) {
                // SAFETY: the original descriptor/instance still owns this buffer.
                assert_eq!(
                    unsafe { self.0.api.release_buffer.unwrap()(self.0.instance, self.1) },
                    OK
                );
            }
        }
        let _release = Release(self, output.id);
        output.validate(DEFAULT_REPLY_BYTES).unwrap();
        // SAFETY: the producing instance retains these immutable bytes until Release drops.
        let bytes =
            unsafe { std::slice::from_raw_parts(output.bytes.data, output.bytes.len as usize) };
        decode_response(bytes, DEFAULT_REPLY_BYTES as usize).unwrap()
    }
    pub fn resolve(&self, path: &Path) -> (u64, Node) {
        let Response::Resolved { mount, node } = self.call(Request::Resolve {
            address: address(path),
        }) else {
            panic!("Native resolve failed")
        };
        (mount, node)
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        // SAFETY: calls/reply buffers quiesce before this owner; native state closes on the backend test thread.
        assert_eq!(unsafe { self.api.destroy.unwrap()(self.instance) }, OK);
    }
}
/// Create a directory junction at the fresh fixture `path` targeting `target`,
/// in-process and with the names `mklink /J` writes. Junctions need no symlink
/// privilege; the caller's fixture owns all cleanup.
pub fn junction(path: &Path, target: &Path) {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::{
        Storage::FileSystem::{FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT},
        System::{
            IO::DeviceIoControl, Ioctl::FSCTL_SET_REPARSE_POINT,
            SystemServices::IO_REPARSE_TAG_MOUNT_POINT,
        },
    };
    let absolute: Vec<u16> = std::path::absolute(target)
        .unwrap()
        .as_os_str()
        .encode_wide()
        .collect();
    let verbatim: Vec<u16> = r"\\?\".encode_utf16().collect();
    let print = absolute
        .strip_prefix(verbatim.as_slice())
        .unwrap_or(&absolute);
    let substitute: Vec<u16> = r"\??\"
        .encode_utf16()
        .chain(print.iter().copied())
        .collect();
    std::fs::create_dir(path).unwrap();
    let directory = std::fs::OpenOptions::new()
        .write(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .unwrap();
    // REPARSE_DATA_BUFFER: tag, data length and reserved, then the mount-point
    // name offsets/lengths in bytes and both NUL-terminated names.
    let bytes = |units: &[u16]| u16::try_from(units.len() * 2).unwrap();
    let data_length = 8 + bytes(&substitute) + 2 + bytes(print) + 2;
    let mut buffer = Vec::new();
    buffer.extend(IO_REPARSE_TAG_MOUNT_POINT.to_le_bytes());
    for value in [
        data_length,
        0,
        0,
        bytes(&substitute),
        bytes(&substitute) + 2,
        bytes(print),
    ] {
        buffer.extend(value.to_le_bytes());
    }
    for unit in substitute.iter().chain(&[0]).chain(print).chain(&[0]) {
        buffer.extend(unit.to_le_bytes());
    }
    let mut returned = 0;
    // SAFETY: the owned directory handle and the complete input buffer remain
    // live through this synchronous control call; no output buffer is used.
    let set = unsafe {
        DeviceIoControl(
            directory.as_raw_handle(),
            FSCTL_SET_REPARSE_POINT,
            buffer.as_ptr().cast(),
            u32::try_from(buffer.len()).unwrap(),
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    assert_ne!(
        set,
        0,
        "junction fixture {}: {}",
        path.display(),
        std::io::Error::last_os_error()
    );
}
pub fn address(path: &Path) -> Address {
    Address {
        encoding: AddressEncoding::WindowsUtf16Le,
        bytes: path
            .as_os_str()
            .encode_wide()
            .flat_map(u16::to_le_bytes)
            .collect(),
    }
}
pub struct Temp {
    pub path: PathBuf,
    root: PathBuf,
}
impl Temp {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let base = repository.join("_build/goal/native-plugin/windows-fixtures");
        std::fs::create_dir_all(&base).unwrap();
        let root = base.canonicalize().unwrap();
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = base.join(format!("fixture-{}-{id}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self { path, root }
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let resolved = self.path.canonicalize().unwrap();
        assert_eq!(resolved.parent(), Some(self.root.as_path()));
        assert!(resolved.starts_with(&self.root) && resolved != self.root);
        std::fs::remove_dir_all(&self.path).unwrap();
    }
}
