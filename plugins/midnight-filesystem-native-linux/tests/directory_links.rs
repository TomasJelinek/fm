#![cfg(target_os = "linux")]
mod support;
use midnight_filesystem_plugin_api::*;
use std::{
    ffi::OsString,
    fs,
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::{MetadataExt, symlink},
    },
};
use support::*;

#[test]
fn listed_directory_link_preserves_link_metadata_and_revalidates_for_navigation() {
    let temp = Temp::new();
    let target = temp.path.join("target 資料");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("child"), b"target bytes").unwrap();
    let name = OsString::from_vec(vec![b'l', 0xff]);
    let link = temp.path.join(&name);
    symlink(&target, &link).unwrap();
    let before = fs::symlink_metadata(&link).unwrap();
    let client = Client::new();
    let (mount, root) = client.resolve(&temp.path);
    let Response::Page {
        cursor,
        entries,
        done,
        ..
    } = client.call(Request::ListBegin {
        mount,
        directory: root.id,
        max_entries: 100,
    })
    else {
        panic!("listing failed")
    };
    assert!(done);
    let listed = entries
        .iter()
        .find(|node| node.raw_name == name.as_bytes())
        .unwrap();
    assert_eq!(listed.kind, Kind::DirectoryLink);
    assert_eq!(listed.address, address(&link));
    assert_eq!(listed.metadata.unix_mode, Some(before.mode()));
    assert_eq!(
        listed.metadata.unix_mode.unwrap() & libc::S_IFMT,
        libc::S_IFLNK
    );
    assert_eq!(listed.metadata.size, None);
    assert!(
        matches!(
            client.call(Request::OpenRead {
                mount,
                node: listed.id
            }),
            Response::Failed(Failure {
                code: ErrorCode::Unsupported,
                ..
            })
        ),
        "directory-link classification does not grant content-read authority"
    );
    assert_eq!(client.call(Request::CloseList { cursor }), Response::Unit);
    let Response::Node(current) = client.call(Request::Stat {
        mount,
        node: listed.id,
    }) else {
        panic!("listed link stat failed")
    };
    assert_eq!(current.kind, listed.kind);
    assert_eq!(current.metadata.unix_mode, listed.metadata.unix_mode);
    assert_eq!(current.address, listed.address);
    let Response::Page {
        cursor, entries, ..
    } = client.call(Request::ListBegin {
        mount,
        directory: listed.id,
        max_entries: 100,
    })
    else {
        panic!("listed directory link must pass SDK kind revalidation")
    };
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].raw_name, b"child");
    assert_eq!(client.call(Request::CloseList { cursor }), Response::Unit);
    assert_eq!(client.call(Request::CloseMount { mount }), Response::Unit);
}

#[test]
fn file_broken_and_looping_links_remain_links_without_target_metadata() {
    let temp = Temp::new();
    fs::write(temp.path.join("target-file"), b"file bytes").unwrap();
    for (name, target) in [
        ("file-link", "target-file"),
        ("broken-link", "missing"),
        ("loop-link", "loop-link"),
    ] {
        symlink(target, temp.path.join(name)).unwrap();
    }
    let client = Client::new();
    let (mount, root) = client.resolve(&temp.path);
    let Response::Page {
        cursor, entries, ..
    } = client.call(Request::ListBegin {
        mount,
        directory: root.id,
        max_entries: 100,
    })
    else {
        panic!("listing failed")
    };
    for name in ["file-link", "broken-link", "loop-link"] {
        let listed = entries
            .iter()
            .find(|entry| entry.raw_name == name.as_bytes())
            .unwrap();
        assert_eq!(listed.kind, Kind::Symlink, "{name}");
        assert_eq!(listed.metadata.size, None);
        assert_eq!(
            listed.metadata.unix_mode.unwrap() & libc::S_IFMT,
            libc::S_IFLNK
        );
        let (other, resolved) = client.resolve(&temp.path.join(name));
        assert_eq!(resolved.kind, Kind::Symlink);
        assert_eq!(
            client.call(Request::CloseMount { mount: other }),
            Response::Unit
        );
    }
    assert_eq!(client.call(Request::CloseList { cursor }), Response::Unit);
    assert_eq!(client.call(Request::CloseMount { mount }), Response::Unit);
}
