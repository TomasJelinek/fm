#![cfg(target_os = "macos")]
#![allow(unsafe_code)]
mod support;
use midnight_filesystem_plugin_api::*;
use std::{
    fs::File,
    io::Write,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::ffi::OsStrExt,
    },
};
use support::*;

#[test]
fn component_relative_listing_stat_and_read_exceed_darwin_path_max() {
    let fixture = Fixture::new();
    let mut current = File::open(&fixture.path).unwrap();
    let mut parents = Vec::new();
    let mut path = fixture.path.clone();
    let component = c"long-component-0123456789-abcdefghijklmnopqrstuvwxyz";
    while path.as_os_str().as_bytes().len() < 2200 {
        // SAFETY: fresh child beneath an owned directory, one terminated name and fixed mode.
        assert_eq!(
            unsafe { libc::mkdirat(current.as_raw_fd(), component.as_ptr(), 0o700) },
            0
        );
        // SAFETY: existing fixture directory, no creation flags and a single terminated name.
        let fd = unsafe {
            libc::openat(
                current.as_raw_fd(),
                component.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        assert!(fd >= 0);
        parents.push(current);
        // SAFETY: successful openat returned exactly one new owned descriptor.
        current = unsafe { File::from_raw_fd(fd) };
        path.push(std::ffi::OsStr::from_bytes(component.to_bytes()));
    }
    // SAFETY: exclusive new file, single fixture name and explicit owner-only mode.
    let fd = unsafe {
        libc::openat(
            current.as_raw_fd(),
            c"source".as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC,
            0o600,
        )
    };
    assert!(fd >= 0);
    // SAFETY: successful openat transfers one descriptor to the File owner.
    let mut file = unsafe { File::from_raw_fd(fd) };
    file.write_all(b"long Darwin content").unwrap();
    drop(file);
    let client = Client::new();
    let (mount, node) = client.resolve(&path.join("source"));
    let reader = client.open(mount, node.id);
    assert_eq!(
        client.call(Request::ReadAt {
            reader,
            offset: 0,
            length: 256
        }),
        Response::Content {
            bytes: b"long Darwin content".to_vec(),
            eof: true
        }
    );
    assert_eq!(client.call(Request::CloseRead { reader }), Response::Unit);
    assert_eq!(client.call(Request::CloseMount { mount }), Response::Unit);
    let (mount, node) = client.resolve(&path);
    let Response::Page {
        cursor, entries, ..
    } = client.call(Request::ListBegin {
        mount,
        directory: node.id,
        max_entries: 4,
    })
    else {
        panic!("Long directory listing failed")
    };
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].raw_name, b"source");
    assert_eq!(client.call(Request::CloseList { cursor }), Response::Unit);
    assert_eq!(client.call(Request::CloseMount { mount }), Response::Unit);
    // Clean through the same retained fixture descriptors rather than a whole long pathname.
    // SAFETY: fixed fixture file name relative to the retained deepest directory.
    assert_eq!(
        unsafe { libc::unlinkat(current.as_raw_fd(), c"source".as_ptr(), 0) },
        0
    );
    while let Some(parent) = parents.pop() {
        drop(current);
        // SAFETY: fixed empty fixture directory, retained parent and directory-only deletion.
        assert_eq!(
            unsafe { libc::unlinkat(parent.as_raw_fd(), component.as_ptr(), libc::AT_REMOVEDIR) },
            0
        );
        current = parent;
    }
}
