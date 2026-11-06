use midnight_core::{ShortcutBinding, ShortcutChord, ShortcutContext, ShortcutMap};

#[test]
fn viewer_space_does_not_capture_search_field_text() {
    let map = ShortcutMap::new(&[]).unwrap();
    let space = ShortcutChord::parse("Space").unwrap();
    assert_eq!(
        map.resolve(&space, ShortcutContext::Viewer).unwrap().id,
        "viewer.next-file"
    );
    assert!(map.resolve(&space, ShortcutContext::ViewerSearch).is_none());
    let escape = ShortcutChord::parse("Escape").unwrap();
    assert_eq!(
        map.resolve(&escape, ShortcutContext::ViewerSearch)
            .unwrap()
            .id,
        "viewer.close"
    );
}

#[test]
fn view_commands_have_distinct_panel_and_viewer_meanings() {
    let map = ShortcutMap::new(&[]).unwrap();
    let f3 = ShortcutChord::parse("F3").unwrap();
    assert_eq!(
        map.resolve(&f3, ShortcutContext::Files).unwrap().id,
        "files.view"
    );
    assert_eq!(
        map.resolve(&f3, ShortcutContext::Viewer).unwrap().id,
        "viewer.find-next"
    );
}

#[test]
fn viewer_search_preserves_native_editing_when_shortcuts_are_customized() {
    for key in ["Space", "X", "Ctrl+C", "Left", "Enter"] {
        assert!(
            ShortcutMap::new(&[ShortcutBinding {
                action: "viewer.close".into(),
                keys: vec![key.into()],
            }])
            .is_err(),
            "{key} must remain native in the search field"
        );
    }
    assert!(
        ShortcutMap::new(&[ShortcutBinding {
            action: "viewer.close".into(),
            keys: vec!["Escape".into()],
        }])
        .is_ok()
    );
}
