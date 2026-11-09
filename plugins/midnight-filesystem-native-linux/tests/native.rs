#![cfg(target_os = "linux")]
#![allow(unsafe_code)]
mod support;
use midnight_filesystem_plugin_api::*;
use std::{
    ffi::{CString, OsString},
    fs::{self, File, FileTimes},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::{OsStrExt, OsStringExt},
            fs::symlink,
        },
    },
    path::Path,
};
use support::*;

#[test]
fn public_table_streams_raw_names_and_captures_selected_content() {
    let temp = Temp::new();
    let path = temp.path.join("selected 文档");
    let expected = b"native Linux \0\xff";
    fs::write(&path, expected).unwrap();
    let name = OsString::from_vec(vec![b'x', 0xff]);
    fs::write(temp.path.join(&name), b"other").unwrap();
    for number in 0..7 {
        fs::write(temp.path.join(format!("entry{number}")), b"unselected").unwrap();
    }
    let client = Client::new();
    let (mount, root) = client.resolve(&temp.path);
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
            panic!("Listing failed")
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
    assert!(
        all.iter().any(|node| node.raw_name == name.as_bytes()
            && node.name_encoding == NameEncoding::UnixBytes)
    );
    let node = all
        .iter()
        .find(|node| node.address == address(&path))
        .unwrap();
    assert!(node.metadata.native_attributes.is_none());
    let Response::Opened { reader, length, .. } = client.call(Request::OpenRead {
        mount,
        node: node.id,
    }) else {
        panic!("Capture failed")
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
    assert_eq!(
        client.cancelled(Request::CloseRead { reader }, 1),
        Response::Unit
    );
    assert_eq!(client.call(Request::CloseList { cursor }), Response::Unit);
    assert_eq!(client.call(Request::CloseMount { mount }), Response::Unit);
}

#[test]
fn replacement_with_same_length_and_modified_time_cannot_retarget_an_old_reader() {
    let temp = Temp::new();
    let path = temp.path.join("source");
    fs::write(&path, b"original").unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
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
    let replacement = temp.path.join("replacement");
    fs::write(&replacement, b"replaced").unwrap();
    File::options()
        .write(true)
        .open(&replacement)
        .unwrap()
        .set_times(FileTimes::new().set_modified(modified))
        .unwrap();
    fs::rename(&replacement, &path).unwrap();
    assert!(matches!(
        client.call(Request::ValidateRead { reader }),
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
fn symlinks_and_fifos_are_not_read_as_content_and_pseudo_files_do_not_fake_zero_length() {
    let temp = Temp::new();
    let source = temp.path.join("source");
    fs::write(&source, b"outside sentinel").unwrap();
    let link = temp.path.join("link");
    symlink(&source, &link).unwrap();
    let fifo = temp.path.join("fifo");
    let name = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    // SAFETY: exclusive fresh fixture path and fixed owner-only FIFO mode.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let client = Client::new();
    for path in [&link, &fifo] {
        let (mount, node) = client.resolve(path);
        assert!(matches!(
            client.call(Request::OpenRead {
                mount,
                node: node.id
            }),
            Response::Failed(Failure {
                code: ErrorCode::Unsupported,
                ..
            })
        ));
    }
    assert_eq!(fs::read(&source).unwrap(), b"outside sentinel");
    let (mount, node) = client.resolve(Path::new("/proc/self/cmdline"));
    assert!(matches!(
        client.call(Request::OpenRead {
            mount,
            node: node.id
        }),
        Response::Failed(Failure {
            code: ErrorCode::Unsupported,
            ..
        })
    ));
    // Trailing slash/dot requires a directory at the kernel boundary.
    for bytes in [
        format!("{}/", source.display()),
        format!("{}/.", source.display()),
    ] {
        assert!(matches!(
            client.call(Request::Resolve {
                address: Address {
                    encoding: AddressEncoding::UnixBytes,
                    bytes: bytes.into_bytes()
                }
            }),
            Response::Failed(_)
        ));
    }
}

#[test]
fn descriptor_relative_access_reads_a_path_longer_than_four_thousand_bytes() {
    let temp = Temp::new();
    let mut directory = File::open(&temp.path).unwrap();
    let mut long = temp.path.clone();
    let component = CString::new("long-native-component-文档-0123456789").unwrap();
    while long.as_os_str().as_bytes().len() < 4300 {
        // SAFETY: owned current directory, fixed single native component, owner-only mode.
        assert_eq!(
            unsafe { libc::mkdirat(directory.as_raw_fd(), component.as_ptr(), 0o700) },
            0
        );
        // SAFETY: existing owned directory, terminated single name, no O_CREAT.
        let fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                component.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        assert!(fd >= 0);
        // SAFETY: successful openat produced one new owned descriptor.
        directory = unsafe { File::from_raw_fd(fd) };
        long = long.join("long-native-component-文档-0123456789");
    }
    // SAFETY: new file is relative to the owned deepest directory; fixed mode accompanies O_CREAT.
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            c"source".as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC,
            0o600,
        )
    };
    assert!(fd >= 0);
    // SAFETY: successful exclusive creation produced one newly owned descriptor.
    let mut file = unsafe { File::from_raw_fd(fd) };
    use std::io::Write;
    file.write_all(b"long path bytes").unwrap();
    drop(file);
    let path = long.join("source");
    let client = Client::new();
    let (mount, node) = client.resolve(&path);
    assert_eq!(node.address, address(&path));
    let Response::Opened { reader, .. } = client.call(Request::OpenRead {
        mount,
        node: node.id,
    }) else {
        panic!("Long path capture failed")
    };
    assert_eq!(
        client.call(Request::ReadAt {
            reader,
            offset: 0,
            length: 256
        }),
        Response::Content {
            bytes: b"long path bytes".to_vec(),
            eof: true
        }
    );
    let (folder, root) = client.resolve(&long);
    let Response::Page {
        cursor,
        entries,
        done,
        ..
    } = client.call(Request::ListBegin {
        mount: folder,
        directory: root.id,
        max_entries: 128,
    })
    else {
        panic!("Long directory failed")
    };
    assert!(done);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].raw_name, b"source");
    assert_eq!(client.call(Request::CloseList { cursor }), Response::Unit);
}

#[test]
fn old_directory_token_rejects_a_new_symlink_but_explicit_link_navigation_works() {
    let temp = Temp::new();
    let path = temp.path.join("directory");
    let target = temp.path.join("target");
    fs::create_dir(&path).unwrap();
    fs::create_dir(&target).unwrap();
    fs::write(target.join("selected"), b"target").unwrap();
    let client = Client::new();
    let (mount, directory) = client.resolve(&path);
    fs::rename(&path, temp.path.join("old-directory")).unwrap();
    symlink(&target, &path).unwrap();
    assert!(matches!(
        client.call(Request::ListBegin {
            mount,
            directory: directory.id,
            max_entries: 10
        }),
        Response::Failed(Failure {
            code: ErrorCode::SourceChanged,
            ..
        })
    ));
    assert_eq!(client.call(Request::CloseMount { mount }), Response::Unit);
    let (mount, link) = client.resolve(&path);
    assert_eq!(link.kind, Kind::Symlink);
    let Response::Page {
        cursor, entries, ..
    } = client.call(Request::ListBegin {
        mount,
        directory: link.id,
        max_entries: 10,
    })
    else {
        panic!("Explicit link navigation");
    };
    assert_eq!(entries[0].raw_name, b"selected");
    assert_eq!(client.call(Request::CloseList { cursor }), Response::Unit);
    assert_eq!(client.call(Request::CloseMount { mount }), Response::Unit);
}
