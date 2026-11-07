use midnight_core::{Bookmark, BookmarkId, UserPreferences};
use std::path::PathBuf;

fn bookmark() -> Bookmark {
    Bookmark {
        id: BookmarkId(1),
        name: "文档 e\u{301} 🧪".into(),
        path: std::env::current_dir().unwrap().join("字".repeat(40_000)),
        slot: Some(10),
    }
}

#[test]
fn native_bookmarks_preserve_unicode_and_do_not_impose_a_path_length_limit() {
    let preferences = UserPreferences {
        bookmarks: vec![bookmark()],
        ..Default::default()
    };
    preferences.validate().unwrap();
    assert!(preferences.bookmarks[0].path.as_os_str().len() > 32_768);
}

#[test]
fn malformed_id_name_slot_path_and_duplicate_bookmarks_are_rejected() {
    let valid = bookmark();
    let mut cases = Vec::new();
    let mut bad = valid.clone();
    bad.id = BookmarkId(0);
    cases.push(bad);
    let mut bad = valid.clone();
    bad.name = "bad\nname".into();
    cases.push(bad);
    let mut bad = valid.clone();
    bad.slot = Some(0);
    cases.push(bad);
    let mut bad = valid.clone();
    bad.path = PathBuf::from("relative");
    cases.push(bad);
    let mut bad = valid.clone();
    bad.path = std::env::current_dir().unwrap().join("bad\0name");
    cases.push(bad);
    for bad in cases {
        assert!(
            UserPreferences {
                bookmarks: vec![bad],
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
    let mut preferences = UserPreferences {
        bookmarks: vec![valid.clone(), valid],
        ..Default::default()
    };
    assert!(preferences.validate().is_err());
    preferences.bookmarks[1].id = BookmarkId(2);
    assert!(
        preferences.validate().is_err(),
        "slot cannot belong to two bookmarks"
    );
    preferences.bookmarks[1].slot = None;
    assert!(
        preferences.validate().is_err(),
        "the same native path cannot be saved twice"
    );
    preferences.bookmarks[1].path = std::env::current_dir().unwrap().join("other");
    assert!(preferences.validate().is_ok());
}
