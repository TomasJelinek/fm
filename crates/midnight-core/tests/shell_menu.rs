use midnight_core::{
    ShellMenuModifiers, ShellMenuPosition, ShellMenuRequest, ShellMenuTarget, ShellMenuVerb,
};
use std::path::PathBuf;

fn request(paths: Vec<PathBuf>) -> ShellMenuRequest {
    ShellMenuRequest {
        paths,
        owner: 0,
        position: ShellMenuPosition { x: -1530, y: 480 },
        modifiers: ShellMenuModifiers::default(),
        target: ShellMenuTarget::Items,
    }
}

#[test]
fn shell_menu_validation_preserves_native_selection_and_negative_screen_position() {
    let root = std::env::current_dir().unwrap();
    let paths = vec![root.join("文档 & $file.txt"), root.join("folder's name")];
    let menu = request(paths.clone());
    assert!(menu.validate().is_ok());
    assert_eq!(menu.paths, paths);
    assert_eq!(menu.position.x, -1530);
}

#[test]
fn shell_menu_rejects_empty_relative_mixed_parent_and_multiple_drive_targets() {
    let root = std::env::current_dir().unwrap();
    for paths in [
        vec![],
        vec![PathBuf::from("relative.txt")],
        vec![root.join("first/a.txt"), root.join("second/b.txt")],
    ] {
        assert!(request(paths).validate().is_err());
    }
    let mut drive = request(vec![root.clone(), root.join("another")]);
    drive.target = ShellMenuTarget::Drive;
    assert!(drive.validate().is_err());
}

#[test]
fn shell_menu_resource_limit_rejects_the_whole_selection() {
    let root = std::env::current_dir().unwrap();
    let paths = (0..4097).map(|n| root.join(format!("item-{n}"))).collect();
    let menu = request(paths);
    assert!(menu.validate().is_err());
    assert_eq!(menu.paths.len(), 4097);
    assert!(
        request(vec![root.join("x".repeat(4 * 1024 * 1024))])
            .validate()
            .is_err()
    );
}

#[test]
fn shell_menu_managed_verbs_use_canonical_names_only() {
    for (name, verb) in [
        ("copy", ShellMenuVerb::Copy),
        ("CUT", ShellMenuVerb::Cut),
        ("Paste", ShellMenuVerb::Paste),
        ("delete", ShellMenuVerb::Delete),
        ("rename", ShellMenuVerb::Rename),
    ] {
        assert_eq!(ShellMenuVerb::from_canonical(name), Some(verb));
    }
    for name in ["", "paste link", "pastelink", "Kopieren", "copyto", " copy"] {
        assert_eq!(ShellMenuVerb::from_canonical(name), None);
    }
}
