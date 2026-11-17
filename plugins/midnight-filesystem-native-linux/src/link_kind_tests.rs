//! Listing, Stat and Resolve give a link the same kind through the exported C
//! ABI, so the SDK's kind revalidation before navigation passes and the target's
//! own error is reported. An offline automount is simulated by failing only the
//! directory open of the link, which is the call that would mount it.
use crate::{
    rename_test_hooks as hooks,
    rename_test_support::{Client, Temp},
};
use midnight_filesystem_plugin_api::*;
use std::{fs, os::unix::fs::symlink};

#[test]
fn listing_and_stat_classify_links_alike_and_navigation_reports_the_target_error() {
    let _reset = hooks::reset();
    let temp = Temp::new();
    fs::create_dir(temp.path.join("directory")).unwrap();
    fs::write(temp.path.join("file"), b"file bytes").unwrap();
    symlink("directory", temp.path.join("directory-link")).unwrap();
    symlink("missing", temp.path.join("dangling-link")).unwrap();
    symlink("file", temp.path.join("file-link")).unwrap();
    hooks::fail_directory_open(&temp.path.join("directory-link"), libc::EHOSTDOWN);
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
    for (name, kind, errno) in [
        ("directory-link", Kind::DirectoryLink, libc::EHOSTDOWN),
        ("dangling-link", Kind::Symlink, libc::ENOENT),
        ("file-link", Kind::Symlink, libc::ENOTDIR),
    ] {
        let listed = entries
            .iter()
            .find(|entry| entry.raw_name == name.as_bytes())
            .unwrap();
        assert_eq!(listed.kind, kind, "{name}");
        let response = client.call(Request::Stat {
            mount,
            node: listed.id,
        });
        assert!(
            matches!(&response, Response::Node(current) if current.kind == kind),
            "{name}: {response:?}"
        );
        let (other, resolved) = client.resolve(&temp.path.join(name));
        assert_eq!(resolved.kind, kind, "{name}");
        assert_eq!(
            client.call(Request::CloseMount { mount: other }),
            Response::Unit
        );
        let response = client.call(Request::ListBegin {
            mount,
            directory: listed.id,
            max_entries: 100,
        });
        assert!(
            matches!(&response, Response::Failed(failure) if failure.os_code == Some(errno)),
            "{name} must report its target's error, not SourceChanged: {response:?}"
        );
    }
    assert_eq!(client.call(Request::CloseList { cursor }), Response::Unit);
    assert_eq!(client.call(Request::CloseMount { mount }), Response::Unit);
}
