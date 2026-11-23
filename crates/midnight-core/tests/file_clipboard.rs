use midnight_core::{FileClipboard, FileClipboardIntent, MAX_FILE_CLIPBOARD_BYTES};
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
    let oversized: [&dyn Fn() -> Vec<PathBuf>; 4] = [
        &Vec::new,
        &|| vec![PathBuf::from("relative.txt")],
        &|| vec![root.join("x".repeat(MAX_FILE_CLIPBOARD_BYTES))],
        // Enough paths for their names to exceed the payload's bytes.
        &|| {
            (0..=MAX_FILE_CLIPBOARD_BYTES / (1 << 20))
                .map(|n| root.join(format!("{n}-{}", "y".repeat(1 << 20))))
                .collect()
        },
    ];
    for paths in oversized {
        let clipboard = FileClipboard {
            paths: paths(),
            intent: FileClipboardIntent::Copy,
        };
        assert!(clipboard.validate().is_err());
    }
}

#[test]
fn file_clipboard_takes_any_number_of_paths_within_its_payload_bytes() {
    // Lists of more than 4,096 paths were once refused; only the names' bytes bound a
    // list now, as the native formats have no count limit either.
    let root = std::env::current_dir().unwrap();
    let clipboard = FileClipboard {
        paths: (0..10_000).map(|n| root.join(n.to_string())).collect(),
        intent: FileClipboardIntent::Copy,
    };
    clipboard.validate().unwrap();
}
