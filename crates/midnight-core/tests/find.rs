use midnight_core::*;

#[test]
fn find_default_preserves_saved_alt_f7_assignments() {
    let chord = ShortcutChord::parse("Alt+F7").unwrap();
    let defaults = ShortcutMap::new(&[]).unwrap();
    assert_eq!(
        defaults.resolve(&chord, ShortcutContext::Files).unwrap().id,
        "files.find"
    );
    let existing = ShortcutBinding {
        action: "files.properties".into(),
        keys: vec!["Alt+F7".into()],
    };
    let migrated = ShortcutMap::new(std::slice::from_ref(&existing)).unwrap();
    assert_eq!(
        migrated.resolve(&chord, ShortcutContext::Files).unwrap().id,
        "files.properties"
    );
    assert!(migrated.bindings("files.find").unwrap().is_empty());
    assert!(
        ShortcutMap::new(&[
            existing,
            ShortcutBinding {
                action: "files.find".into(),
                keys: vec!["Alt+F7".into()],
            },
        ])
        .is_err()
    );
}

#[test]
fn find_defaults_match_literal_recursive_native_search_and_validate_limits() {
    let mut request = FindRequest::new(
        FindSessionId(1),
        FindGeneration(1),
        vec![std::env::current_dir().unwrap()],
    );
    assert!(request.recursive);
    assert_eq!(request.masks, "*.*");
    assert!(!request.name_case_sensitive);
    assert!(request.content.is_none());
    assert!(request.validate().is_ok());
    request.limits.max_results = 0;
    assert!(request.validate().is_err());
}

#[test]
fn find_rejects_ambiguous_roots_and_empty_or_unbounded_content_queries() {
    let mut request =
        FindRequest::new(FindSessionId(1), FindGeneration(1), vec!["relative".into()]);
    assert!(request.validate().is_err());
    request.roots = vec![std::env::current_dir().unwrap()];
    request.content = Some(FindContentQuery {
        text: String::new(),
        case_sensitive: false,
    });
    assert!(request.validate().is_err());
    request.content.as_mut().unwrap().text = "č".repeat(4097);
    assert!(request.validate().is_err());
}
