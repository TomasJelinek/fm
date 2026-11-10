#![cfg(target_os = "macos")]
#![allow(unsafe_code)]
mod support;
use midnight_filesystem_plugin_api::*;
use std::{
    ffi::{CString, OsString},
    fs::{self, File, FileTimes},
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::symlink,
    },
    path::Path,
};
use support::*;

#[test]
fn pages_keep_kernel_names_and_selected_file_bytes() {
    let fixture = Fixture::new();
    let selected = fixture.path.join("selected 文档-é");
    fs::write(&selected, b"selected\0\xff").unwrap();
    for number in 0..7 {
        fs::write(fixture.path.join(format!("entry{number}")), b"other").unwrap();
    }
    let raw = fixture.path.join(OsString::from_vec(b"raw-\xff".to_vec()));
    // Filesystems may themselves refuse non-UTF-8 names. Every name the kernel
    // does return must retain its actual bytes, including normalization choices.
    if let Err(error) = fs::write(&raw, b"raw") {
        assert!(matches!(
            error.raw_os_error(),
            Some(libc::EINVAL | libc::EILSEQ)
        ));
    }
    let expected: std::collections::HashSet<_> = fs::read_dir(&fixture.path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().as_bytes().to_vec())
        .collect();
    let client = Client::new();
    let (mount, root) = client.resolve(&fixture.path);
    let mut response = client.call(Request::ListBegin {
        mount,
        directory: root.id,
        max_entries: 2,
    });
    let mut actual = std::collections::HashSet::new();
    let mut page_number = 0;
    let cursor = loop {
        let Response::Page {
            cursor,
            page,
            done,
            entries,
        } = response
        else {
            panic!("Bad listing")
        };
        assert_eq!(page, page_number);
        assert!(entries.len() <= 2);
        for entry in entries {
            assert_eq!(entry.name_encoding, NameEncoding::UnixBytes);
            assert_eq!(
                entry.address.bytes,
                fixture
                    .path
                    .join(OsString::from_vec(entry.raw_name.clone()))
                    .as_os_str()
                    .as_bytes()
            );
            actual.insert(entry.raw_name);
        }
        if done {
            break cursor;
        }
        page_number += 1;
        response = client.call(Request::ListNext {
            cursor,
            expected_page: page_number,
            max_entries: 2,
        });
    };
    assert_eq!(actual, expected);
    assert_eq!(client.call(Request::CloseList { cursor }), Response::Unit);
    assert_eq!(client.call(Request::CloseMount { mount }), Response::Unit);
    let (mount, node) = client.resolve(&selected);
    let reader = client.open(mount, node.id);
    assert_eq!(
        client.call(Request::ReadAt {
            reader,
            offset: 0,
            length: 256
        }),
        Response::Content {
            bytes: b"selected\0\xff".to_vec(),
            eof: true
        }
    );
    assert_eq!(
        client.call(Request::ReadAt {
            reader,
            offset: 10,
            length: 1
        }),
        Response::Content {
            bytes: vec![],
            eof: true
        }
    );
    assert_eq!(
        client.cancelled(Request::CloseRead { reader }, true),
        Response::Unit
    );
    assert_eq!(client.call(Request::CloseMount { mount }), Response::Unit);
}

#[test]
fn replaced_or_edited_content_never_validates_as_the_original_capture() {
    let fixture = Fixture::new();
    let path = fixture.path.join("source");
    fs::write(&path, b"original").unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let client = Client::new();
    let (mount, node) = client.resolve(&path);
    let reader = client.open(mount, node.id);
    let replacement = fixture.path.join("replacement");
    fs::write(&replacement, b"replaced").unwrap();
    File::options()
        .write(true)
        .open(&replacement)
        .unwrap()
        .set_times(FileTimes::new().set_modified(modified))
        .unwrap();
    fs::rename(replacement, &path).unwrap();
    failure(
        client.call(Request::ValidateRead { reader }),
        ErrorCode::SourceChanged,
    );
    failure(
        client.call(Request::ReadAt {
            reader,
            offset: 0,
            length: 8,
        }),
        ErrorCode::SourceChanged,
    );
    assert_eq!(client.call(Request::CloseRead { reader }), Response::Unit);
    let fresh = client.open(mount, node.id);
    fs::write(&path, b"modified").unwrap();
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(modified + std::time::Duration::from_secs(2)))
        .unwrap();
    failure(
        client.call(Request::ValidateRead { reader: fresh }),
        ErrorCode::SourceChanged,
    );
    assert_eq!(
        client.call(Request::CloseRead { reader: fresh }),
        Response::Unit
    );
    assert_eq!(client.call(Request::CloseMount { mount }), Response::Unit);
}

#[test]
fn links_and_special_entries_are_metadata_only_until_explicit_directory_navigation() {
    let fixture = Fixture::new();
    let target = fixture.path.join("target");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("sentinel"), b"untouched").unwrap();
    let link = fixture.path.join("directory-link");
    symlink(&target, &link).unwrap();
    let fifo = fixture.path.join("fifo");
    let native = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    // SAFETY: a fresh owned path and owner-only mode; no existing endpoint is touched.
    assert_eq!(unsafe { libc::mkfifo(native.as_ptr(), 0o600) }, 0);
    let client = Client::new();
    for path in [&link, &fifo, Path::new("/dev/null")] {
        let (mount, node) = client.resolve(path);
        failure(
            client.call(Request::OpenRead {
                mount,
                node: node.id,
            }),
            ErrorCode::Unsupported,
        );
        assert_eq!(client.call(Request::CloseMount { mount }), Response::Unit);
    }
    let (mount, node) = client.resolve(&link);
    assert_eq!(node.kind, Kind::Symlink);
    let Response::Page {
        cursor, entries, ..
    } = client.call(Request::ListBegin {
        mount,
        directory: node.id,
        max_entries: 4,
    })
    else {
        panic!("Explicit link navigation failed")
    };
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].raw_name, b"sentinel");
    assert_eq!(client.call(Request::CloseList { cursor }), Response::Unit);
    assert_eq!(client.call(Request::CloseMount { mount }), Response::Unit);
    assert_eq!(fs::read(target.join("sentinel")).unwrap(), b"untouched");
}

#[test]
fn cancellation_and_native_trailing_components_preserve_boundaries() {
    let fixture = Fixture::new();
    let file = fixture.path.join("file");
    fs::write(&file, b"content").unwrap();
    let client = Client::new();
    failure(
        client.cancelled(
            Request::Resolve {
                address: address(&file),
            },
            true,
        ),
        ErrorCode::Cancelled,
    );
    for suffix in [b"/".as_slice(), b"/.", b"/../another"] {
        let mut bytes = file.as_os_str().as_bytes().to_vec();
        bytes.extend(suffix);
        assert!(matches!(
            client.call(Request::Resolve {
                address: Address {
                    encoding: AddressEncoding::UnixBytes,
                    bytes
                }
            }),
            Response::Failed(_)
        ));
    }
    let (mount, node) = client.resolve(&file);
    let reader = client.open(mount, node.id);
    failure(
        client.cancelled(
            Request::ReadAt {
                reader,
                offset: 0,
                length: 1,
            },
            true,
        ),
        ErrorCode::Cancelled,
    );
    assert_eq!(
        client.cancelled(Request::CloseRead { reader }, true),
        Response::Unit
    );
    assert_eq!(
        client.cancelled(Request::CloseMount { mount }, true),
        Response::Unit
    );
}
