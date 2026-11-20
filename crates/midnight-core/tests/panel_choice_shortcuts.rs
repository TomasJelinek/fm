//! Show hidden files (Ctrl+H), select/deselect by mask (keypad + and −, or Ctrl+=
//! and Ctrl+−) and the keypad's invert.
use midnight_core::*;

fn action(map: &ShortcutMap, keys: &str, context: ShortcutContext) -> Option<String> {
    map.resolve(&ShortcutChord::parse(keys).unwrap(), context)
        .map(|action| action.id.clone())
}

#[test]
fn keypad_names_parse_round_trip_and_differ_from_the_typed_characters() {
    for (text, canonical) in [
        ("NumpadAdd", "NumpadAdd"),
        ("numplus", "NumpadAdd"),
        ("KeypadPlus", "NumpadAdd"),
        ("NumpadSubtract", "NumpadSubtract"),
        ("NumMinus", "NumpadSubtract"),
        ("NumpadMultiply", "NumpadMultiply"),
        ("numstar", "NumpadMultiply"),
        ("Ctrl+NumpadAdd", "NumpadAdd"),
    ] {
        let chord = ShortcutChord::parse(text).unwrap();
        assert_eq!(chord.key, canonical, "{text}");
        assert_eq!(ShortcutChord::parse(&chord.display()).unwrap(), chord);
    }
    for typed in ["Plus", "Minus", "*"] {
        assert!(
            !ShortcutChord::parse(typed)
                .unwrap()
                .key
                .starts_with("Numpad")
        );
    }
}

#[test]
fn defaults_select_by_mask_on_the_keypad_and_toggle_hidden_files_with_control_h() {
    let map = ShortcutMap::new(&[]).unwrap();
    for (keys, id) in [
        ("NumpadAdd", "selection.mask-select"),
        ("NumpadSubtract", "selection.mask-deselect"),
        ("NumpadMultiply", "selection.mask-invert"),
        // Without a numeric keypad: Ctrl+= (Ctrl++ where + needs no Shift) and Ctrl+−.
        ("Ctrl+Equal", "selection.mask-select"),
        ("Ctrl+Plus", "selection.mask-select"),
        ("Ctrl+Minus", "selection.mask-deselect"),
        ("Ctrl+H", "panel.hidden-files"),
    ] {
        assert_eq!(
            action(&map, keys, ShortcutContext::Files).as_deref(),
            Some(id),
            "{keys}"
        );
    }
    // Menus show the key every keyboard has first.
    assert_eq!(
        map.bindings("selection.mask-select").unwrap()[0],
        ShortcutChord::parse("Ctrl+Equal").unwrap()
    );
    assert_eq!(
        map.bindings("selection.mask-deselect").unwrap()[0],
        ShortcutChord::parse("Ctrl+Minus").unwrap()
    );
    // Ctrl+Shift+= still bookmarks the folder, and the viewer keeps its zoom keys.
    for (keys, id) in [
        ("Ctrl+Shift+Plus", "bookmark.add"),
        ("Ctrl+Shift+Equal", "bookmark.add"),
    ] {
        assert_eq!(
            action(&map, keys, ShortcutContext::Files).as_deref(),
            Some(id)
        );
    }
    for (keys, id) in [
        ("Ctrl+Equal", "viewer.zoom-in"),
        ("Ctrl+Minus", "viewer.zoom-out"),
    ] {
        assert_eq!(
            action(&map, keys, ShortcutContext::Viewer).as_deref(),
            Some(id)
        );
    }
    // During filename search the keypad keys type into the query, and none of the
    // mask keys act there or in the command line; Ctrl+H still works.
    for keys in [
        "NumpadAdd",
        "NumpadSubtract",
        "NumpadMultiply",
        "Ctrl+Equal",
        "Ctrl+Plus",
        "Ctrl+Minus",
    ] {
        assert_eq!(action(&map, keys, ShortcutContext::Search), None, "{keys}");
        assert_eq!(action(&map, keys, ShortcutContext::Command), None, "{keys}");
    }
    assert_eq!(
        action(&map, "Ctrl+H", ShortcutContext::Search).as_deref(),
        Some("panel.hidden-files")
    );
    // The number row's characters stay text: nothing selects by mask.
    for keys in ["Plus", "Minus", "*", "Shift+Plus"] {
        assert_eq!(action(&map, keys, ShortcutContext::Files), None, "{keys}");
    }
    for id in [
        "selection.mask-select",
        "selection.mask-deselect",
        "selection.mask-invert",
        "panel.hidden-files",
    ] {
        assert!(
            shortcut_catalog()
                .iter()
                .any(|definition| definition.id == id),
            "{id} is configurable in Options › Keyboard"
        );
    }
}

#[test]
fn new_defaults_yield_to_saved_assignments_and_can_be_rebound_or_removed() {
    // A user who had already assigned Ctrl+H, a keypad key or Ctrl+− keeps that
    // binding; only the taken key leaves the new action.
    let saved = ShortcutMap::new(&[
        ShortcutBinding {
            action: "panel.refresh".into(),
            keys: vec!["Ctrl+H".into()],
        },
        ShortcutBinding {
            action: "selection.all".into(),
            keys: vec!["NumpadAdd".into()],
        },
        ShortcutBinding {
            action: "selection.clear-files".into(),
            keys: vec!["Ctrl+Minus".into()],
        },
    ])
    .unwrap();
    assert_eq!(
        action(&saved, "Ctrl+H", ShortcutContext::Files).as_deref(),
        Some("panel.refresh")
    );
    assert_eq!(
        action(&saved, "NumpadAdd", ShortcutContext::Files).as_deref(),
        Some("selection.all")
    );
    assert_eq!(
        action(&saved, "Ctrl+Minus", ShortcutContext::Files).as_deref(),
        Some("selection.clear-files")
    );
    assert!(saved.bindings("panel.hidden-files").unwrap().is_empty());
    assert_eq!(
        saved.bindings("selection.mask-select").unwrap().to_vec(),
        vec![
            ShortcutChord::parse("Ctrl+Equal").unwrap(),
            ShortcutChord::parse("Ctrl+Plus").unwrap()
        ]
    );
    assert_eq!(
        saved.bindings("selection.mask-deselect").unwrap().to_vec(),
        vec![ShortcutChord::parse("NumpadSubtract").unwrap()]
    );

    let rebound = ShortcutMap::new(&[
        ShortcutBinding {
            action: "panel.hidden-files".into(),
            keys: vec!["Ctrl+Period".into()],
        },
        ShortcutBinding {
            action: "selection.mask-invert".into(),
            keys: vec![],
        },
    ])
    .unwrap();
    assert_eq!(
        action(&rebound, "Ctrl+Period", ShortcutContext::Files).as_deref(),
        Some("panel.hidden-files")
    );
    assert_eq!(action(&rebound, "Ctrl+H", ShortcutContext::Files), None);
    assert_eq!(
        action(&rebound, "NumpadMultiply", ShortcutContext::Files),
        None
    );
}

#[test]
fn keypad_keys_cannot_be_taken_from_text_editors() {
    let binding = ShortcutBinding {
        action: "command.execute".into(),
        keys: vec!["NumpadAdd".into()],
    };
    let error = ShortcutMap::new(&[binding]).unwrap_err();
    assert!(
        error.contains("reserved for native text editing"),
        "{error}"
    );
}
