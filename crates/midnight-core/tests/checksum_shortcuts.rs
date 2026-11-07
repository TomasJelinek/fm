use midnight_core::{ShortcutBinding, ShortcutChord, ShortcutContext, ShortcutMap};

#[test]
fn checksums_default_and_custom_binding_preserve_case_conversion() {
    let defaults = ShortcutMap::new(&[]).unwrap();
    for (key, action) in [("Ctrl+F8", "files.checksums"), ("Ctrl+F7", "files.case")] {
        assert_eq!(
            defaults
                .resolve(&ShortcutChord::parse(key).unwrap(), ShortcutContext::Files)
                .unwrap()
                .id,
            action
        );
    }
    let reassigned = ShortcutMap::new(&[ShortcutBinding {
        action: "files.checksums".into(),
        keys: vec!["Ctrl+Alt+H".into()],
    }])
    .unwrap();
    assert!(
        reassigned
            .resolve(
                &ShortcutChord::parse("Ctrl+F8").unwrap(),
                ShortcutContext::Files
            )
            .is_none()
    );
    assert_eq!(
        reassigned
            .resolve(
                &ShortcutChord::parse("Ctrl+Alt+H").unwrap(),
                ShortcutContext::Files
            )
            .unwrap()
            .id,
        "files.checksums"
    );
}

#[test]
fn checksums_new_default_does_not_take_existing_explicit_shortcut() {
    let previous = ShortcutBinding {
        action: "files.properties".into(),
        keys: vec!["Ctrl+F8".into()],
    };
    let chord = ShortcutChord::parse("Ctrl+F8").unwrap();
    let migrated = ShortcutMap::new(std::slice::from_ref(&previous)).unwrap();
    assert_eq!(
        migrated.resolve(&chord, ShortcutContext::Files).unwrap().id,
        "files.properties"
    );
    assert!(migrated.bindings("files.checksums").unwrap().is_empty());
    assert!(
        ShortcutMap::new(&[
            previous,
            ShortcutBinding {
                action: "files.checksums".into(),
                keys: vec!["Ctrl+F8".into()],
            },
        ])
        .is_err()
    );
    let disabled = ShortcutMap::new(&[ShortcutBinding {
        action: "files.checksums".into(),
        keys: vec![],
    }])
    .unwrap();
    assert!(disabled.resolve(&chord, ShortcutContext::Files).is_none());
}
