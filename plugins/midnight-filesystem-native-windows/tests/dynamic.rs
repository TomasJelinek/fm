#![cfg(windows)]
#![allow(unsafe_code)]
mod support;
use midnight_filesystem_plugin_api::{abi::*, *};
use std::os::windows::ffi::OsStrExt;
use support::*;
use windows_sys::Win32::{
    Foundation::{FreeLibrary, HMODULE},
    System::LibraryLoader::*,
};

struct Library(HMODULE);
impl Drop for Library {
    fn drop(&mut self) {
        // SAFETY: test client/calls/buffers are destroyed before this owned loader reference.
        unsafe { FreeLibrary(self.0) };
    }
}
#[test]
fn actual_loaded_dll_owns_descriptor_buffers_and_selected_file_content() {
    let temp = Temp::new();
    let path = temp.path.join("dynamic 文档.bin");
    let expected = b"bytes from the DLL \0\xff";
    std::fs::write(&path, expected).unwrap();
    // Cargo emits the independent package's cdylib beside this integration binary.
    let library = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join("midnight_filesystem_native_windows.dll")
        .canonicalize()
        .unwrap();
    let wide: Vec<_> = library.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: exact owned absolute native path, terminated UTF-16 and restricted dependency search.
    let handle = unsafe {
        LoadLibraryExW(
            wide.as_ptr(),
            std::ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    };
    assert!(!handle.is_null(), "{}", std::io::Error::last_os_error());
    let _library = Library(handle);
    // SAFETY: the live trusted test DLL exports this fixed versioned C signature.
    let symbol = unsafe { GetProcAddress(handle, ENTRY_SYMBOL.as_ptr().cast()) }.unwrap();
    let entry: Entry = unsafe { std::mem::transmute(symbol) };
    let mut api = Api::default();
    // SAFETY: complete writable descriptor capacity, retained library and fixed C entry.
    assert_eq!(unsafe { entry(&mut api, size_of::<Api>() as u32) }, OK);
    api.validate(size_of::<Api>() as u32).unwrap();
    let mut owner = std::ptr::null_mut();
    // SAFETY: FROM_ADDRESS interprets its second parameter as a code address, not a string.
    assert_ne!(
        unsafe {
            GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
                    | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                api.call.unwrap() as *const () as *const u16,
                &mut owner,
            )
        },
        0
    );
    assert_eq!(
        owner, handle,
        "callbacks must belong to the loaded DLL, not a static native fallback"
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
    // SAFETY: descriptor strings belong to the retained loaded DLL.
    assert_eq!(
        unsafe {
            std::slice::from_raw_parts(client.api.version.data, client.api.version.len as usize)
        },
        b"0.2.0"
    );
    let original_identity = file_identity(&path);
    let Response::RenamePrepared(prepared) = client.call(Request::PrepareRename {
        mount,
        node: node.id,
    }) else {
        panic!("Loaded DLL did not prepare Rename")
    };
    assert_eq!(prepared.source.metadata.size, Some(expected.len() as u64));
    assert_eq!(std::fs::read(&path).unwrap(), expected);
    let renamed = temp.path.join("renamed DLL 文档.bin");
    let name = NativeText {
        encoding: NameEncoding::WindowsUtf16Le,
        bytes: renamed
            .file_name()
            .unwrap()
            .encode_wide()
            .flat_map(u16::to_le_bytes)
            .collect(),
    };
    let request = Request::Rename {
        prepared: prepared.handle,
        attempt: 1,
        name: name.clone(),
        policy: RenamePolicy::NoReplace,
    };
    let result = client.call(request.clone());
    let Response::RenameState(state) = &result else {
        panic!("Loaded DLL did not retain Rename outcome")
    };
    assert!(
        matches!(&state.outcome, RenameOutcome::Renamed { name: actual, same_entry: false, recovery, .. } if actual == &name && recovery.is_empty())
    );
    assert!(!path.exists());
    assert_eq!(file_identity(&renamed), original_identity);
    assert_eq!(std::fs::read(&renamed).unwrap(), expected);
    assert_eq!(
        client.cancelled(
            Request::GetRenameState {
                prepared: prepared.handle
            },
            1
        ),
        result
    );
    assert_eq!(client.call(request), result);
    assert_eq!(
        client.cancelled(
            Request::CloseRename {
                prepared: prepared.handle
            },
            1
        ),
        Response::Unit
    );
    assert_eq!(client.call(Request::CloseMount { mount }), Response::Unit);
}

fn file_identity(path: &std::path::Path) -> (u64, [u8; 16]) {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ID_INFO, FileIdInfo, GetFileInformationByHandleEx,
    };
    let file = std::fs::File::open(path).unwrap();
    let mut info = FILE_ID_INFO::default();
    // SAFETY: the owned fixture handle is live and the output has its declared size.
    assert_ne!(
        unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileIdInfo,
                (&mut info as *mut FILE_ID_INFO).cast(),
                size_of::<FILE_ID_INFO>() as u32,
            )
        },
        0
    );
    (info.VolumeSerialNumber, info.FileId.Identifier)
}
