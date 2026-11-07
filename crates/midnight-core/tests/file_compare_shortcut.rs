use midnight_core::{ShortcutBinding, ShortcutChord, ShortcutContext, ShortcutMap};

#[test]
fn file_compare_chord_is_configurable_and_separate_from_copy() {
    let defaults = ShortcutMap::new(&[]).unwrap();
    let chord = ShortcutChord::parse("Ctrl+Shift+C").unwrap();
    assert_eq!(
        defaults.resolve(&chord, ShortcutContext::Files).unwrap().id,
        "files.compare"
    );
    assert_eq!(
        defaults
            .resolve(
                &ShortcutChord::parse("Ctrl+C").unwrap(),
                ShortcutContext::Files
            )
            .unwrap()
            .id,
        "files.copy"
    );
    let changed = ShortcutMap::new(&[ShortcutBinding {
        action: "files.compare".into(),
        keys: vec!["Ctrl+Shift+D".into()],
    }])
    .unwrap();
    assert!(changed.resolve(&chord, ShortcutContext::Files).is_none());
    assert_eq!(
        changed
            .resolve(
                &ShortcutChord::parse("Ctrl+Shift+D").unwrap(),
                ShortcutContext::Files
            )
            .unwrap()
            .id,
        "files.compare"
    );
}

#[test]
fn new_defaults_preserve_existing_overrides_but_explicit_conflicts_fail() {
    for (new_action, old_action, key, context) in [
        (
            "files.compare",
            "files.properties",
            "Ctrl+Shift+C",
            ShortcutContext::Files,
        ),
        (
            "viewer.document-next",
            "viewer.up",
            "Ctrl+PageDown",
            ShortcutContext::Viewer,
        ),
        (
            "viewer.document-previous",
            "viewer.up",
            "Ctrl+PageUp",
            ShortcutContext::Viewer,
        ),
        (
            "compare.next-difference",
            "viewer.wrap",
            "F7",
            ShortcutContext::Viewer,
        ),
        (
            "compare.previous-difference",
            "viewer.wrap",
            "Shift+F7",
            ShortcutContext::Viewer,
        ),
    ] {
        let original = ShortcutBinding {
            action: old_action.into(),
            keys: vec![key.into()],
        };
        let map = ShortcutMap::new(std::slice::from_ref(&original)).unwrap();
        assert_eq!(
            map.resolve(&ShortcutChord::parse(key).unwrap(), context)
                .unwrap()
                .id,
            old_action
        );
        assert!(
            ShortcutMap::new(&[
                original,
                ShortcutBinding {
                    action: new_action.into(),
                    keys: vec![key.into()]
                }
            ])
            .is_err()
        );
    }
}
