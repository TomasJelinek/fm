#![cfg(target_os = "macos")]
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
        // SAFETY: the test drops API clients, requests and buffers before this unique module owner.
        unsafe { libc::dlclose(self.0) };
    }
}

#[test]
fn actual_dylib_exports_its_own_callbacks_and_reads_selected_bytes() {
    let fixture = Fixture::new();
    let selected = fixture.path.join("dynamic 文档");
    std::fs::write(&selected, b"dylib bytes\0\xff").unwrap();
    let path = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join("libmidnight_filesystem_native_macos.dylib")
        .canonicalize()
        .unwrap();
    let native = CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: absolute package-built fixture library, immediate local symbol resolution.
    let handle = unsafe { libc::dlopen(native.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
    assert!(!handle.is_null(), "Cannot load built macOS provider");
    let _library = Library(handle);
    // SAFETY: the retained package exports the exact public ABI entry signature.
    let symbol = unsafe { libc::dlsym(handle, ENTRY_SYMBOL.as_ptr()) };
    assert!(!symbol.is_null());
    let entry: Entry = unsafe { std::mem::transmute(symbol) };
    let mut table = Api::default();
    // SAFETY: complete caller-owned table and live module through all subsequent calls.
    assert_eq!(unsafe { entry(&mut table, size_of::<Api>() as u32) }, OK);
    table.validate(size_of::<Api>() as u32).unwrap();
    // SAFETY: Dl_info is a plain output record and callback points into loaded executable code.
    let mut origin: libc::Dl_info = unsafe { std::mem::zeroed() };
    assert_ne!(
        unsafe { libc::dladdr(table.call.unwrap() as *const () as *const _, &mut origin) },
        0
    );
    assert!(!origin.dli_fname.is_null());
    // SAFETY: dladdr returned a terminated filename whose module remains loaded.
    let name = unsafe { CStr::from_ptr(origin.dli_fname) };
    assert_eq!(
        Path::new(std::ffi::OsStr::from_bytes(name.to_bytes()))
            .canonicalize()
            .unwrap(),
        path
    );
    let client = Client::from_table(table);
    let (mount, node) = client.resolve(&selected);
    let reader = client.open(mount, node.id);
    assert_eq!(
        client.call(Request::ReadAt {
            reader,
            offset: 0,
            length: 256
        }),
        Response::Content {
            bytes: b"dylib bytes\0\xff".to_vec(),
            eof: true
        }
    );
    assert_eq!(client.call(Request::CloseRead { reader }), Response::Unit);
    assert_eq!(client.call(Request::CloseMount { mount }), Response::Unit);
}
