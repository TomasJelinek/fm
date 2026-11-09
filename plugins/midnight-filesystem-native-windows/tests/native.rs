#![cfg(windows)]
#![allow(unsafe_code)]
mod support;
use midnight_filesystem_plugin_api::*;
use std::{
    ffi::OsString,
    fs::{self, File, FileTimes},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        io::AsRawHandle,
    },
};
use support::*;

#[test]
fn public_table_streams_native_names_and_captures_the_selected_regular_file() {
    let temp = Temp::new();
    let expected = b"chosen native bytes \0\xff";
    let selected = temp.path.join("selected 文档.bin");
    fs::write(&selected, expected).unwrap();
    let invalid = OsString::from_wide(&[b'x' as u16, 0xD800]);
    fs::write(temp.path.join(&invalid), b"other").unwrap();
    for number in 0..7 {
        fs::write(temp.path.join(format!("entry{number}")), b"unselected").unwrap();
    }
    let client = Client::new();
    let (mount, root) = client.resolve(&temp.path);
    assert_eq!(root.address, address(&temp.path));
    let mut response = client.call(Request::ListBegin {
        mount,
        directory: root.id,
        max_entries: 2,
    });
    let mut all = vec![];
    let mut expected_page = 0;
    let cursor = loop {
        let Response::Page {
            cursor,
            page,
            done,
            entries,
        } = response
        else {
            panic!("Native page failed")
        };
        assert_eq!(page, expected_page);
        assert!(entries.len() <= 2);
        all.extend(entries);
        if done {
            break cursor;
        }
        expected_page += 1;
        response = client.call(Request::ListNext {
            cursor,
            expected_page,
            max_entries: 2,
        });
    };
    assert_eq!(all.len(), 9);
    let raw: Vec<_> = invalid.encode_wide().flat_map(u16::to_le_bytes).collect();
    assert!(
        all.iter()
            .any(|node| node.raw_name == raw && node.name_encoding == NameEncoding::WindowsUtf16Le)
    );
    let selected = all
        .iter()
        .find(|node| node.address == address(&selected))
        .unwrap();
    let Response::Opened {
        reader,
        length,
        revision,
    } = client.call(Request::OpenRead {
        mount,
        node: selected.id,
    })
    else {
        panic!("Native capture failed")
    };
    assert_eq!(length, expected.len() as u64);
    assert!(!revision.is_empty());
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
    assert_eq!(
        client.cancelled(Request::CloseRead { reader }, 1),
        Response::Unit
    );
    assert_eq!(client.call(Request::CloseList { cursor }), Response::Unit);
    assert_eq!(client.call(Request::CloseMount { mount }), Response::Unit);
}

#[test]
fn same_size_edit_with_restored_modified_time_invalidates_the_retained_source() {
    let temp = Temp::new();
    let path = temp.path.join("source.bin");
    fs::write(&path, b"original").unwrap();
    // Native timestamp resolution can coalesce two updates in one clock tick.
    // Start with an explicitly older reported change time, without sleeping.
    let fixture = File::options().write(true).open(&path).unwrap();
    use windows_sys::Win32::Storage::FileSystem::*;
    let mut basic = FILE_BASIC_INFO::default();
    // SAFETY: owned fixture handle and exact writable FILE_BASIC_INFO capacity.
    assert_ne!(
        unsafe {
            GetFileInformationByHandleEx(
                fixture.as_raw_handle(),
                FileBasicInfo,
                (&mut basic as *mut FILE_BASIC_INFO).cast(),
                size_of::<FILE_BASIC_INFO>() as u32,
            )
        },
        0
    );
    basic.ChangeTime -= 100_000_000;
    // SAFETY: metadata change applies only to this owned temporary fixture.
    assert_ne!(
        unsafe {
            SetFileInformationByHandle(
                fixture.as_raw_handle(),
                FileBasicInfo,
                (&basic as *const FILE_BASIC_INFO).cast(),
                size_of::<FILE_BASIC_INFO>() as u32,
            )
        },
        0
    );
    drop(fixture);
    let client = Client::new();
    let (mount, node) = client.resolve(&path);
    let Response::Opened {
        reader, revision, ..
    } = client.call(Request::OpenRead {
        mount,
        node: node.id,
    })
    else {
        panic!("Capture failed")
    };
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    fs::write(&path, b"replaced").unwrap();
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(modified))
        .unwrap();
    let validation = client.call(Request::ValidateRead { reader });
    let current = client.call(Request::OpenRead {
        mount,
        node: node.id,
    });
    assert!(
        matches!(
            validation,
            Response::Failed(Failure {
                code: ErrorCode::SourceChanged,
                ..
            })
        ),
        "validation={validation:?}; previous={revision:?}; current={current:?}"
    );
    assert!(matches!(
        client.call(Request::ReadAt {
            reader,
            offset: 0,
            length: 8
        }),
        Response::Failed(Failure {
            code: ErrorCode::SourceChanged,
            ..
        })
    ));
    let Response::Opened {
        reader: fresh,
        revision: changed,
        ..
    } = client.call(Request::OpenRead {
        mount,
        node: node.id,
    })
    else {
        panic!("Fresh capture failed")
    };
    assert_ne!(revision, changed);
    assert_ne!(reader, fresh);
    assert_eq!(
        client.call(Request::ReadAt {
            reader: fresh,
            offset: 0,
            length: 8
        }),
        Response::Content {
            bytes: b"replaced".to_vec(),
            eof: true
        }
    );
}

#[test]
fn long_absolute_paths_roundtrip_and_foreign_mount_nodes_are_rejected() {
    let temp = Temp::new();
    let mut long = temp.path.clone();
    for _ in 0..9 {
        long = long.join("long native directory 文档");
    }
    fs::create_dir_all(&long).unwrap();
    let path = long.join("source.bin");
    fs::write(&path, b"long").unwrap();
    assert!(path.as_os_str().encode_wide().count() > 260);
    let client = Client::new();
    let (mount, node) = client.resolve(&path);
    assert_eq!(node.address, address(&path));
    let (other, _) = client.resolve(&temp.path);
    assert!(matches!(
        client.call(Request::Stat {
            mount: other,
            node: node.id
        }),
        Response::Failed(Failure {
            code: ErrorCode::StaleReference,
            ..
        })
    ));
    let Response::Opened { reader, .. } = client.call(Request::OpenRead {
        mount,
        node: node.id,
    }) else {
        panic!("Long capture failed")
    };
    assert_eq!(
        client.call(Request::ReadAt {
            reader,
            offset: 0,
            length: 9
        }),
        Response::Content {
            bytes: b"long".to_vec(),
            eof: true
        }
    );
}

#[test]
fn empty_files_and_cancelled_listing_do_not_invent_data_or_leak_cursor_resources() {
    let temp = Temp::new();
    let path = temp.path.join("empty");
    fs::write(&path, b"").unwrap();
    let client = Client::new();
    let (mount, node) = client.resolve(&path);
    let Response::Opened { reader, length, .. } = client.call(Request::OpenRead {
        mount,
        node: node.id,
    }) else {
        panic!("Empty capture failed")
    };
    assert_eq!(length, 0);
    assert_eq!(
        client.call(Request::ReadAt {
            reader,
            offset: 0,
            length: 1
        }),
        Response::Content {
            bytes: vec![],
            eof: true
        }
    );
    let (folder, root) = client.resolve(&temp.path);
    assert!(matches!(
        client.cancelled(
            Request::ListBegin {
                mount: folder,
                directory: root.id,
                max_entries: 1
            },
            1
        ),
        Response::Failed(Failure {
            code: ErrorCode::Cancelled,
            ..
        })
    ));
    assert_eq!(
        client.call(Request::CloseMount { mount: folder }),
        Response::Unit
    );
    assert!(matches!(
        client.call(Request::OpenRead {
            mount: folder,
            node: root.id
        }),
        Response::Failed(Failure {
            code: ErrorCode::StaleReference,
            ..
        })
    ));
}

#[test]
fn dotted_and_spaced_raw_names_never_read_their_ordinary_sibling() {
    let temp = Temp::new();
    fs::write(temp.path.join("source"), b"ordinary").unwrap();
    for name in ["source.", "source "] {
        let literal = temp.path.canonicalize().unwrap().join(name);
        fs::write(&literal, b"literal!").unwrap();
    }
    let client = Client::new();
    let (mount, root) = client.resolve(&temp.path);
    let Response::Page {
        cursor,
        entries,
        done: true,
        ..
    } = client.call(Request::ListBegin {
        mount,
        directory: root.id,
        max_entries: 10,
    })
    else {
        panic!("List");
    };
    for name in ["source.", "source "] {
        let raw: Vec<_> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let node = entries.iter().find(|node| node.raw_name == raw).unwrap();
        let Response::Opened { reader, .. } = client.call(Request::OpenRead {
            mount,
            node: node.id,
        }) else {
            panic!("Open literal");
        };
        assert_eq!(
            client.call(Request::ReadAt {
                reader,
                offset: 0,
                length: 100
            }),
            Response::Content {
                bytes: b"literal!".to_vec(),
                eof: true
            },
            "{name}"
        );
        assert_eq!(client.call(Request::CloseRead { reader }), Response::Unit);
    }
    assert_eq!(client.call(Request::CloseList { cursor }), Response::Unit);
    assert_eq!(client.call(Request::CloseMount { mount }), Response::Unit);
}
