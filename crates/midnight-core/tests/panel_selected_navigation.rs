use midnight_core::{ShortcutBinding, ShortcutChord, ShortcutContext, ShortcutMap};

fn action(map: &ShortcutMap, key: &str, context: ShortcutContext) -> Option<String> {
    map.resolve(&ShortcutChord::parse(key).unwrap(), context)
        .map(|action| action.id.clone())
}

#[test]
fn selected_navigation_defaults_are_file_only_and_leave_comparison_and_command_history_intact() {
    let map = ShortcutMap::new(&[]).unwrap();
    for (key, panel, comparison, command) in [
        (
            "Alt+Up",
            "panel.previous-selected",
            "compare.previous-difference",
            "command.history-older",
        ),
        (
            "Alt+Down",
            "panel.next-selected",
            "compare.next-difference",
            "command.history-newer",
        ),
    ] {
        assert_eq!(
            action(&map, key, ShortcutContext::Files).as_deref(),
            Some(panel)
        );
        assert_eq!(
            action(&map, key, ShortcutContext::Comparison).as_deref(),
            Some(comparison)
        );
        assert_eq!(
            action(&map, key, ShortcutContext::Command).as_deref(),
            Some(command)
        );
        for context in [
            ShortcutContext::Search,
            ShortcutContext::Other,
            ShortcutContext::Viewer,
            ShortcutContext::ViewerSearch,
            ShortcutContext::ComparisonSearch,
            ShortcutContext::Volume,
        ] {
            assert_eq!(action(&map, key, context), None, "{key} in {context:?}");
        }
    }
    assert_eq!(
        action(&map, "Backspace", ShortcutContext::Files).as_deref(),
        Some("panel.parent")
    );
}

#[test]
fn selected_navigation_supports_remapping_disabling_and_prior_explicit_assignments() {
    let map = ShortcutMap::new(&[
        ShortcutBinding {
            action: "panel.previous-selected".into(),
            keys: vec!["Ctrl+Shift+J".into()],
        },
        ShortcutBinding {
            action: "panel.next-selected".into(),
            keys: vec![],
        },
    ])
    .unwrap();
    assert_eq!(
        action(&map, "Ctrl+Shift+J", ShortcutContext::Files).as_deref(),
        Some("panel.previous-selected")
    );
    assert_eq!(action(&map, "Alt+Up", ShortcutContext::Files), None);
    assert_eq!(action(&map, "Alt+Down", ShortcutContext::Files), None);
    let map = ShortcutMap::new(&[ShortcutBinding {
        action: "panel.parent".into(),
        keys: vec!["Alt+Up".into(), "Alt+Down".into()],
    }])
    .unwrap();
    for key in ["Alt+Up", "Alt+Down"] {
        assert_eq!(
            action(&map, key, ShortcutContext::Files).as_deref(),
            Some("panel.parent")
        );
    }
    assert!(map.bindings("panel.previous-selected").unwrap().is_empty());
    assert!(map.bindings("panel.next-selected").unwrap().is_empty());
}
