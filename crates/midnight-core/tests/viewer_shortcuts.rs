use midnight_core::{ShortcutBinding, ShortcutChord, ShortcutContext, ShortcutMap};

#[test]
fn image_zoom_and_fit_have_distinct_configurable_viewer_actions() {
    let map = ShortcutMap::new(&[]).unwrap();
    for (key, action) in [
        ("Ctrl+Plus", "viewer.zoom-in"),
        ("Ctrl+Equal", "viewer.zoom-in"),
        ("Ctrl+Minus", "viewer.zoom-out"),
        ("Ctrl+Digit0", "viewer.fit"),
        ("Ctrl+Digit1", "viewer.actual-pixels"),
    ] {
        assert_eq!(
            map.resolve(&ShortcutChord::parse(key).unwrap(), ShortcutContext::Viewer)
                .map(|action| action.id.as_str()),
            Some(action)
        );
    }
}

#[test]
fn wrap_shortcut_is_configurable_and_scoped_to_the_viewer() {
    let chord = ShortcutChord::parse("Ctrl+W").unwrap();
    let map = ShortcutMap::new(&[]).unwrap();
    assert_eq!(
        map.resolve(&chord, ShortcutContext::Viewer)
            .map(|action| action.id.as_str()),
        Some("viewer.wrap")
    );
    let customized = ShortcutMap::new(&[ShortcutBinding {
        action: "viewer.wrap".into(),
        keys: vec!["Ctrl+Shift+W".into()],
    }])
    .unwrap();
    assert!(
        customized
            .resolve(&chord, ShortcutContext::Viewer)
            .is_none()
    );
}

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

#[test]
fn previous_and_next_file_aliases_are_configurable_and_preserve_search_editing() {
    let defaults = ShortcutMap::new(&[]).unwrap();
    for (key, action) in [
        ("Backspace", "viewer.previous-file"),
        ("Alt+Left", "viewer.previous-file"),
        ("Space", "viewer.next-file"),
        ("Alt+Right", "viewer.next-file"),
    ] {
        let chord = ShortcutChord::parse(key).unwrap();
        assert_eq!(
            defaults
                .resolve(&chord, ShortcutContext::Viewer)
                .unwrap()
                .id,
            action
        );
        assert!(
            defaults
                .resolve(&chord, ShortcutContext::ViewerSearch)
                .is_none()
        );
    }
    assert_eq!(
        defaults
            .resolve(
                &ShortcutChord::parse("Backspace").unwrap(),
                ShortcutContext::Files
            )
            .unwrap()
            .id,
        "panel.parent"
    );
    let remapped = ShortcutMap::new(&[ShortcutBinding {
        action: "viewer.previous-file".into(),
        keys: vec!["Ctrl+Shift+B".into()],
    }])
    .unwrap();
    for key in ["Backspace", "Alt+Left"] {
        assert!(
            remapped
                .resolve(&ShortcutChord::parse(key).unwrap(), ShortcutContext::Viewer)
                .is_none()
        );
    }
    let chord = ShortcutChord::parse("Ctrl+Shift+B").unwrap();
    assert_eq!(
        remapped
            .resolve(&chord, ShortcutContext::Viewer)
            .unwrap()
            .id,
        "viewer.previous-file"
    );
    assert!(
        remapped
            .resolve(&chord, ShortcutContext::ViewerSearch)
            .is_none()
    );
    let disabled = ShortcutMap::new(&[ShortcutBinding {
        action: "viewer.previous-file".into(),
        keys: vec![],
    }])
    .unwrap();
    assert!(
        disabled
            .resolve(
                &ShortcutChord::parse("Backspace").unwrap(),
                ShortcutContext::Viewer
            )
            .is_none()
    );
}
