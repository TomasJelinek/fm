#![allow(dead_code)]
use midnight_filesystem_plugin_api::{abi::*, *};
use std::{
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
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
                midnight_filesystem_native_linux::midnight_filesystem_entry_v1(
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
        let bytes = encode_request(&request, DEFAULT_REQUEST_BYTES as usize).unwrap();
        let request = Call {
            struct_size: size_of::<Call>() as u32,
            operation: request.operation() as u32,
            payload: Slice {
                data: bytes.as_ptr(),
                len: bytes.len() as u64,
            },
            cancel: Cancellation {
                context: cancel,
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
pub fn address(path: &Path) -> Address {
    Address {
        encoding: AddressEncoding::UnixBytes,
        bytes: path.as_os_str().as_bytes().to_vec(),
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
        let base = std::env::var_os("MIDNIGHT_NATIVE_FIXTURE_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| repository.join("_build/goal/native-plugin/linux-fixtures"));
        assert!(base.is_absolute() && base.components().any(|part| part.as_os_str() == "_build"));
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

const ACCESS_ACL: &std::ffi::CStr = c"system.posix_acl_access";

fn terminated(path: &Path) -> std::ffi::CString {
    std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap()
}

/// Lets another user (nobody) write to `path` through a POSIX access ACL:
/// `user::rwx user:<other>:rwx group::r-x mask::rwx other::r-x`, so the mode reads
/// 0775 like an ordinary private-group folder. Prints SKIP and returns false where
/// the filesystem has no POSIX ACLs.
pub fn share_through_access_acl(path: &Path, test: &str) -> bool {
    // SAFETY: geteuid has no arguments or ownership requirements.
    let caller = unsafe { libc::geteuid() };
    let other = if caller == 65534 { 65533 } else { 65534u32 };
    // The kernel's posix_acl_xattr format: version 2, then (tag, permissions, id)
    // entries in tag order; ids of the unnamed entries are undefined (u32::MAX).
    let mut acl = 2u32.to_le_bytes().to_vec();
    for (tag, permissions, id) in [
        (0x01u16, 7u16, u32::MAX),
        (0x02, 7, other),
        (0x04, 5, u32::MAX),
        (0x10, 7, u32::MAX),
        (0x20, 5, u32::MAX),
    ] {
        acl.extend_from_slice(&tag.to_le_bytes());
        acl.extend_from_slice(&permissions.to_le_bytes());
        acl.extend_from_slice(&id.to_le_bytes());
    }
    let path = terminated(path);
    // SAFETY: terminated path and attribute name; the value outlives the call.
    let set = unsafe {
        libc::setxattr(
            path.as_ptr(),
            ACCESS_ACL.as_ptr(),
            acl.as_ptr().cast(),
            acl.len(),
            0,
        )
    };
    if set == 0 {
        return true;
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::EOPNOTSUPP) {
        eprintln!("SKIP {test}: the fixture filesystem has no POSIX ACLs");
        return false;
    }
    panic!("setting a POSIX access ACL failed: {error}");
}

/// Removes the access ACL; the mode keeps the former mask as its group bits.
pub fn remove_access_acl(path: &Path) {
    let path = terminated(path);
    // SAFETY: terminated path and attribute name.
    let removed = unsafe { libc::removexattr(path.as_ptr(), ACCESS_ACL.as_ptr()) };
    assert_eq!(removed, 0, "{}", std::io::Error::last_os_error());
}
