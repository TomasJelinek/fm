#![cfg(windows)]
#![allow(unsafe_code)]
mod support;

use midnight_filesystem_plugin_api::*;
use std::{fs, os::windows::ffi::OsStrExt, path::PathBuf};
use support::*;
use windows_sys::Win32::Storage::FileSystem::*;

#[link(name = "ntdll")]
unsafe extern "system" {
    fn RtlSetThreadPlaceholderCompatibilityMode(mode: i8) -> i8;
}

struct ExposedPlaceholders(i8);
impl ExposedPlaceholders {
    fn new() -> Self {
        // SAFETY: changes only the test thread's documented placeholder visibility mode.
        let previous = unsafe { RtlSetThreadPlaceholderCompatibilityMode(2) };
        assert!(previous >= 0);
        Self(previous)
    }
}
impl Drop for ExposedPlaceholders {
    fn drop(&mut self) {
        // SAFETY: restore the exact prior mode on the same test thread.
        unsafe { RtlSetThreadPlaceholderCompatibilityMode(self.0) };
    }
}

// Explicit native acceptance only: the caller chooses one already resident PNG.
// This never creates, modifies, hydrates a placeholder, or enumerates a cloud folder.
#[test]
#[ignore = "requires MIDNIGHT_CLOUD_TEST_FILE pointing to one resident cloud PNG"]
fn resident_cloud_png_is_readable_through_the_public_provider_table() {
    let _mode = ExposedPlaceholders::new();
    let path = PathBuf::from(std::env::var_os("MIDNIGHT_CLOUD_TEST_FILE").unwrap());
    assert!(path.is_absolute());
    let metadata = fs::symlink_metadata(&path).unwrap();
    assert!(metadata.is_file());
    let wide: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: the exact caller-selected absolute name is terminated UTF-16.
    // Querying attributes does not open or hydrate the cloud content stream.
    let attributes = unsafe { GetFileAttributesW(wide.as_ptr()) };
    assert_ne!(attributes, INVALID_FILE_ATTRIBUTES);
    assert_ne!(attributes & FILE_ATTRIBUTE_REPARSE_POINT, 0);
    assert_eq!(
        attributes
            & (FILE_ATTRIBUTE_OFFLINE
                | FILE_ATTRIBUTE_RECALL_ON_OPEN
                | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS),
        0,
        "acceptance must use an already resident file"
    );
    let client = Client::new();
    let (mount, node) = client.resolve(&path);
    assert_eq!(node.kind, Kind::File);
    let response = client.call(Request::OpenRead {
        mount,
        node: node.id,
    });
    let Response::Opened {
        reader,
        length,
        revision,
    } = response
    else {
        panic!("resident cloud file capture failed: {response:?}");
    };
    assert_eq!(length, metadata.len());
    assert!(!revision.is_empty());
    let Response::Content { bytes, eof } = client.call(Request::ReadAt {
        reader,
        offset: 0,
        length: 24,
    }) else {
        panic!("resident cloud header read failed");
    };
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(bytes.len(), 24);
    assert!(!eof);
    assert_eq!(
        client.call(Request::ValidateRead { reader }),
        Response::Unit
    );
    assert_eq!(client.call(Request::CloseRead { reader }), Response::Unit);
    assert_eq!(client.call(Request::CloseMount { mount }), Response::Unit);
}

#[test]
fn content_policy_rejects_links_and_replacement_without_returning_other_bytes() {
    let temp = Temp::new();
    let source = temp.path.join("source.bin");
    let target = temp.path.join("other-folder");
    fs::create_dir(&target).unwrap();
    let outside = target.join("other.bin");
    let link = temp.path.join("link.bin");
    fs::write(&source, b"original").unwrap();
    fs::write(&outside, b"not selected").unwrap();
    junction(&link, &target);
    let client = Client::new();
    let (link_mount, link_node) = client.resolve(&link);
    assert!(matches!(
        client.call(Request::OpenRead {
            mount: link_mount,
            node: link_node.id
        }),
        Response::Failed(_)
    ));
    let (mount, node) = client.resolve(&source);
    let Response::Opened { reader, .. } = client.call(Request::OpenRead {
        mount,
        node: node.id,
    }) else {
        panic!("regular fixture must remain readable");
    };
    fs::rename(&source, temp.path.join("retained.bin")).unwrap();
    junction(&source, &target);
    assert!(matches!(
        client.call(Request::ValidateRead { reader }),
        Response::Failed(Failure {
            code: ErrorCode::SourceChanged,
            ..
        })
    ));
    assert!(matches!(
        client.call(Request::ReadAt {
            reader,
            offset: 0,
            length: 8
        }),
        Response::Failed(_)
    ));
    assert_eq!(fs::read(&outside).unwrap(), b"not selected");
    assert_eq!(client.call(Request::CloseRead { reader }), Response::Unit);
}

fn junction(path: &std::path::Path, target: &std::path::Path) {
    // Junction creation needs no symlink privilege. Both names are fresh owned
    // fixture paths; only creation uses cmd, and Rust owns all cleanup.
    let output = std::process::Command::new("cmd.exe")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(path.to_str().unwrap().replace('/', "\\"))
        .arg(target.to_str().unwrap().replace('/', "\\"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
