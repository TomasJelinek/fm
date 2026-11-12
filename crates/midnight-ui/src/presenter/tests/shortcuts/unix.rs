use super::*;
use std::{ffi::OsString, os::unix::ffi::OsStringExt};

#[test]
fn root_shortcut_keeps_unix_root_for_mounts_and_non_utf8_paths() {
    let path = PathBuf::from(OsString::from_vec(b"/mnt/volume/raw-\xff/child".to_vec()));
    let mut p = Presenter::new(path, test_root());
    assert!(p.go_root(PaneId::Left, &|command| {
        assert!(matches!(command, Command::Navigate { path, .. } if path == Path::new("/")));
        true
    }));
}

#[test]
fn focused_name_preserves_native_non_utf8_bytes() {
    let raw = OsString::from_vec(b"raw-\xff.txt".to_vec());
    let path = test_root().join(&raw);
    let mut p = Presenter::new(test_root(), test_root());
    let navigation = begin(&mut p, PaneId::Left, "/");
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        vec![Entry::new(EntryId(8), path, EntryKind::File)],
    );
    assert_eq!(p.focused_name(PaneId::Left), Some(PathBuf::from(raw)));
}

#[test]
fn tree_command_insertion_preserves_non_utf8_bytes_and_refuses_lossy_editor_text() {
    let raw = OsString::from_vec(b"raw-\xff.txt".to_vec());
    for pane in [PaneId::Left, PaneId::Right] {
        for kind in [EntryKind::Directory, EntryKind::File] {
            let (p, path) = acknowledged_native_tree(pane, kind, &raw);
            let name = p
                .focused_name(pane)
                .expect("the raw basename must be retained");
            let full = p
                .focused_full_path(pane)
                .expect("the raw native path must be retained");
            assert_eq!(name, PathBuf::from(&raw));
            assert_eq!(full, path);
            assert!(name.to_str().is_none());
            assert!(full.to_str().is_none());
            assert!(crate::command_line::command_path_text(&name).is_err());
            assert!(crate::command_line::command_path_text(&full).is_err());
        }
    }
}
