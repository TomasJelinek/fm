use super::*;
use std::{ffi::OsString, os::windows::ffi::OsStringExt};

#[test]
fn root_shortcut_preserves_drive_unc_and_verbatim_anchors() {
    for (input, expected) in [
        (r"D:\nested\folder", r"D:\"),
        (r"\\server\share\nested\folder", r"\\server\share\"),
        (r"\\?\D:\nested\folder", r"\\?\D:\"),
        (r"\\?\UNC\server\share\nested", r"\\?\UNC\server\share\"),
    ] {
        let mut p = Presenter::new(input.into(), test_root());
        assert!(p.go_root(PaneId::Left, &|command| {
            assert!(matches!(command, Command::Navigate { path, .. }
                if path == Path::new(expected)));
            true
        }));
    }
}

#[test]
fn root_shortcut_does_not_resolve_drive_relative_or_root_relative_paths() {
    for input in [r"C:folder", r"\folder"] {
        let mut p = Presenter::new(input.into(), test_root());
        assert!(!p.go_root(PaneId::Left, &|_| panic!(
            "ambiguous root must not navigate"
        )));
        assert_eq!(p.panes[0].display_path(), Path::new(input));
    }
}

#[test]
fn focused_name_preserves_unpaired_native_utf16() {
    let raw = OsString::from_wide(&[0xd800, 0x002e, 0x0074, 0x0078, 0x0074]);
    let path = test_root().join(&raw);
    let mut p = Presenter::new(test_root(), test_root());
    let navigation = begin(&mut p, PaneId::Left, r"C:\");
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        vec![Entry::new(EntryId(8), path, EntryKind::File)],
    );
    assert_eq!(p.focused_name(PaneId::Left), Some(PathBuf::from(raw)));
}

#[test]
fn tree_command_insertion_preserves_unpaired_utf16_and_refuses_lossy_editor_text() {
    let raw = OsString::from_wide(&[0x0072, 0xd800, 0x002e, 0x0074, 0x0078, 0x0074]);
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
