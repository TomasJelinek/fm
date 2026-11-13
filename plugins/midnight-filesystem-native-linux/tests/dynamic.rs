#![cfg(target_os = "linux")]
#![allow(unsafe_code)]
mod support;
use midnight_filesystem_plugin_api::{abi::*, *};
use std::{
    ffi::{CStr, CString},
    os::unix::ffi::OsStrExt,
    path::Path,
};
use support::*;
struct Library(*mut std::ffi::c_void);
impl Drop for Library {
    fn drop(&mut self) {
        // SAFETY: all clients/calls/buffers close before this unique dlopen reference.
        unsafe { libc::dlclose(self.0) };
    }
}
#[test]
fn actual_loaded_shared_library_owns_its_callbacks_and_content() {
    let temp = Temp::new();
    let path = temp.path.join("dynamic 文档");
    let expected = b"bytes from loaded shared library \0\xff";
    std::fs::write(&path, expected).unwrap();
    let library = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join("libmidnight_filesystem_native_linux.so")
        .canonicalize()
        .unwrap();
    let native = CString::new(library.as_os_str().as_bytes()).unwrap();
    // SAFETY: explicit owned absolute library path, immediate resolution, no global symbols.
    let handle = unsafe { libc::dlopen(native.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
    assert!(!handle.is_null());
    let _library = Library(handle);
    // SAFETY: the retained trusted test module exports this exact C entry signature.
    let symbol = unsafe { libc::dlsym(handle, ENTRY_SYMBOL.as_ptr()) };
    assert!(!symbol.is_null());
    let entry: Entry = unsafe { std::mem::transmute(symbol) };
    let mut api = Api::default();
    // SAFETY: complete caller-owned output table and retained library.
    assert_eq!(unsafe { entry(&mut api, size_of::<Api>() as u32) }, OK);
    api.validate(size_of::<Api>() as u32).unwrap();
    // SAFETY: Dl_info is a plain native output record and the callback is mapped code.
    let mut owner: libc::Dl_info = unsafe { std::mem::zeroed() };
    assert_ne!(
        unsafe { libc::dladdr(api.call.unwrap() as *const () as *const _, &mut owner) },
        0
    );
    assert!(!owner.dli_fname.is_null());
    // SAFETY: dladdr's filename is terminated and borrowed while the module remains loaded.
    let origin = unsafe { CStr::from_ptr(owner.dli_fname) };
    let origin = std::ffi::OsStr::from_bytes(origin.to_bytes());
    assert_eq!(
        Path::new(origin).canonicalize().unwrap(),
        library,
        "callback must come from the loaded package, not static native code"
    );
    let client = Client::from_api(api);
    assert_ne!(client.api.capabilities & CAP_VOLUME_DETAILS, 0);
    let Response::VolumeInformation(info) = client.call(Request::VolumeInformation {
        address: address(&temp.path),
    }) else {
        panic!("Loaded module did not provide rich volume information")
    };
    assert!(info.capacity.total.is_some_and(|total| total > 0));
    assert!(info.filesystem.is_some());
    assert_eq!(
        client.call(Request::VolumeLabel {
            address: address(&temp.path)
        }),
        Response::VolumeLabel(info.label)
    );
    let (mount, node) = client.resolve(&path);
    let Response::Opened { reader, length, .. } = client.call(Request::OpenRead {
        mount,
        node: node.id,
    }) else {
        panic!("Dynamic capture failed")
    };
    assert_eq!(length, expected.len() as u64);
    assert_eq!(
        client.call(Request::ReadAt {
            reader,
            offset: 0,
            length: 256
        }),
        Response::Content {
            bytes: expected.to_vec(),
            eof: true
        }
    );
    assert_eq!(client.call(Request::CloseRead { reader }), Response::Unit);

    assert_ne!(client.api.capabilities & CAP_PREPARED_RENAME, 0);
    assert_eq!(client.api.header.minimum_host_minor, 3);
    // SAFETY: the loaded module retains this immutable bounded descriptor string.
    assert_eq!(
        unsafe {
            std::slice::from_raw_parts(client.api.version.data, client.api.version.len as usize)
        },
        b"0.2.0"
    );
    use std::os::unix::fs::MetadataExt;
    let source_metadata = std::fs::symlink_metadata(&path).unwrap();
    let Response::RenamePrepared(prepared) = client.call(Request::PrepareRename {
        mount,
        node: node.id,
    }) else {
        panic!("Loaded module did not prepare Rename");
    };
    assert!(matches!(
        client.call(Request::CloseMount { mount }),
        Response::Failed(Failure {
            code: ErrorCode::Busy,
            ..
        })
    ));
    let new_name = NativeText {
        encoding: NameEncoding::UnixBytes,
        bytes: b"dynamic-renamed-\xff".to_vec(),
    };
    let Response::RenameState(state) = client.call(Request::Rename {
        prepared: prepared.handle,
        attempt: 1,
        name: new_name.clone(),
        policy: RenamePolicy::NoReplace,
    }) else {
        panic!("Loaded module did not execute Rename");
    };
    assert!(
        matches!(&state.outcome, RenameOutcome::Renamed {
        name, same_entry: false, recovery, durability_confirmed: true,
    } if *name == new_name && recovery.is_empty()),
        "{state:?}"
    );
    let destination = temp.path.join(std::ffi::OsStr::from_bytes(&new_name.bytes));
    let published = std::fs::symlink_metadata(&destination).unwrap();
    assert_eq!(
        (published.dev(), published.ino()),
        (source_metadata.dev(), source_metadata.ino())
    );
    assert!(!path.exists());
    assert_eq!(std::fs::read(&destination).unwrap(), expected);
    assert_eq!(
        client.call(Request::GetRenameState {
            prepared: prepared.handle
        }),
        Response::RenameState(state)
    );
    assert_eq!(
        client.call(Request::CloseRename {
            prepared: prepared.handle
        }),
        Response::Unit
    );
    assert_eq!(client.call(Request::CloseMount { mount }), Response::Unit);
}
