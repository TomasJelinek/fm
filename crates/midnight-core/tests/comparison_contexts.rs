use midnight_core::{ShortcutBinding, ShortcutChord, ShortcutContext, ShortcutMap};

fn action(map: &ShortcutMap, key: &str, context: ShortcutContext) -> Option<String> {
    map.resolve(&ShortcutChord::parse(key).unwrap(), context)
        .map(|definition| definition.id.clone())
}

#[test]
fn viewer_file_cycling_and_comparison_differences_have_distinct_contexts() {
    let map = ShortcutMap::new(&[]).unwrap();
    for (key, viewer, comparison) in [
        (
            "Alt+Left",
            "viewer.previous-file",
            "compare.previous-difference",
        ),
        ("Alt+Right", "viewer.next-file", "compare.next-difference"),
    ] {
        assert_eq!(
            action(&map, key, ShortcutContext::Viewer).as_deref(),
            Some(viewer)
        );
        assert_eq!(
            action(&map, key, ShortcutContext::Comparison).as_deref(),
            Some(comparison)
        );
        for context in [
            ShortcutContext::ViewerSearch,
            ShortcutContext::ComparisonSearch,
        ] {
            assert_eq!(action(&map, key, context), None);
        }
    }
    for key in ["Space", "Backspace"] {
        assert_eq!(action(&map, key, ShortcutContext::Comparison), None);
        assert_eq!(action(&map, key, ShortcutContext::ComparisonSearch), None);
    }
    for context in [
        ShortcutContext::Viewer,
        ShortcutContext::ViewerSearch,
        ShortcutContext::Comparison,
        ShortcutContext::ComparisonSearch,
    ] {
        assert_eq!(
            action(&map, "Ctrl+F", context).as_deref(),
            Some("viewer.find")
        );
        assert_eq!(
            action(&map, "Escape", context).as_deref(),
            Some("viewer.close")
        );
        assert_eq!(action(&map, "Ctrl+Tab", context), None);
    }
}

#[test]
fn configured_viewer_and_comparison_aliases_do_not_conflict_across_windows() {
    let bindings = [
        ShortcutBinding {
            action: "viewer.previous-file".into(),
            keys: vec!["Ctrl+J".into()],
        },
        ShortcutBinding {
            action: "compare.previous-difference".into(),
            keys: vec!["Ctrl+J".into()],
        },
    ];
    let map = ShortcutMap::new(&bindings).unwrap();
    assert_eq!(
        action(&map, "Ctrl+J", ShortcutContext::Viewer).as_deref(),
        Some("viewer.previous-file")
    );
    assert_eq!(
        action(&map, "Ctrl+J", ShortcutContext::Comparison).as_deref(),
        Some("compare.previous-difference")
    );
    let mut conflict = bindings.to_vec();
    conflict.push(ShortcutBinding {
        action: "viewer.zoom-in".into(),
        keys: vec!["Ctrl+J".into()],
    });
    assert!(
        ShortcutMap::new(&conflict).is_err(),
        "shared controls still detect real conflicts"
    );
}
