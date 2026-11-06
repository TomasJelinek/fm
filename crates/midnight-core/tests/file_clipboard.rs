use midnight_core::{FileClipboard, FileClipboardIntent};
use std::path::PathBuf;

#[test]
fn file_clipboard_keeps_absolute_paths_and_copy_or_move_intent() {
    let root = std::env::current_dir().unwrap();
    let paths = vec![root.join("first/文档 & a.txt"), root.join("second/b.txt")];
    for intent in [FileClipboardIntent::Copy, FileClipboardIntent::Move] {
        let clipboard = FileClipboard {
            paths: paths.clone(),
            intent,
        };
        assert!(clipboard.validate().is_ok());
        assert_eq!(clipboard.paths, paths);
        assert_eq!(clipboard.intent, intent);
    }
}

#[test]
fn file_clipboard_rejects_empty_relative_and_oversized_payloads_whole() {
    let root = std::env::current_dir().unwrap();
    for paths in [
        Vec::new(),
        vec![PathBuf::from("relative.txt")],
        vec![root.join("x".repeat(4 * 1024 * 1024))],
        (0..4097).map(|n| root.join(n.to_string())).collect(),
    ] {
        let clipboard = FileClipboard {
            paths,
            intent: FileClipboardIntent::Copy,
        };
        assert!(clipboard.validate().is_err());
    }
}
