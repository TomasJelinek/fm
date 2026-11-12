use midnight_core::*;

#[test]
fn selection_shortcuts_distinguish_files_and_folders_for_select_clear_and_invert() {
    let map = ShortcutMap::new(&[]).unwrap();
    for context in [ShortcutContext::Files, ShortcutContext::Search] {
        for (key, expected) in [
            ("Ctrl+A", "selection.files"),
            ("Ctrl+Shift+A", "selection.all"),
            ("Ctrl+D", "selection.clear"),
            ("Ctrl+I", "selection.invert"),
            ("Ctrl+Shift+I", "selection.invert-all"),
        ] {
            assert_eq!(
                action(&map, key, context).as_deref(),
                Some(expected),
                "{key}"
            );
        }
    }
    assert_eq!(action(&map, "Ctrl+Shift+D", ShortcutContext::Files), None);
    assert_eq!(action(&map, "Ctrl+Shift+D", ShortcutContext::Search), None);
    assert!(map.bindings("selection.clear-files").unwrap().is_empty());
    assert_eq!(
        map.bindings("selection.files").unwrap()[0].display(),
        "Ctrl+A"
    );
    for context in [
        ShortcutContext::Command,
        ShortcutContext::Other,
        ShortcutContext::Viewer,
    ] {
        assert_eq!(action(&map, "Ctrl+Shift+A", context), None);
        assert_eq!(action(&map, "Ctrl+Shift+I", context), None);
    }
    let map = ShortcutMap::new(&[ShortcutBinding {
        action: "selection.files".into(),
        keys: vec!["Ctrl+Shift+J".into()],
    }])
    .unwrap();
    assert_eq!(
        action(&map, "Ctrl+Shift+A", ShortcutContext::Files).as_deref(),
        Some("selection.all")
    );
    assert_eq!(
        action(&map, "Ctrl+Shift+J", ShortcutContext::Files).as_deref(),
        Some("selection.files")
    );
    let legacy = ShortcutMap::new(&[ShortcutBinding {
        action: "files.rename".into(),
        keys: vec!["Ctrl+Shift+A".into()],
    }])
    .unwrap();
    assert_eq!(
        action(&legacy, "Ctrl+Shift+A", ShortcutContext::Files).as_deref(),
        Some("files.rename")
    );
}

#[test]
fn selection_new_default_aliases_preserve_explicit_assignments_and_hints() {
    let bindings = [
        ShortcutBinding {
            action: "selection.files".into(),
            keys: vec!["Ctrl+Shift+A".into()],
        },
        ShortcutBinding {
            action: "files.rename".into(),
            keys: vec!["Ctrl+Shift+I".into()],
        },
    ];
    let map = ShortcutMap::new(&bindings).unwrap();
    assert_eq!(
        action(&map, "Ctrl+Shift+A", ShortcutContext::Files).as_deref(),
        Some("selection.files")
    );
    assert_eq!(
        action(&map, "Ctrl+Shift+I", ShortcutContext::Files).as_deref(),
        Some("files.rename")
    );
    assert_eq!(
        map.bindings("selection.all")
            .unwrap()
            .iter()
            .map(ShortcutChord::display)
            .collect::<Vec<_>>(),
        Vec::<String>::new()
    );
    assert!(map.bindings("selection.invert-all").unwrap().is_empty());
    assert!(
        ShortcutMap::new(&[
            bindings[0].clone(),
            ShortcutBinding {
                action: "files.rename".into(),
                keys: vec!["Ctrl+Shift+A".into()]
            },
        ])
        .is_err(),
        "two explicit assignments must still conflict"
    );
}

#[test]
fn comparison_navigation_defaults_are_configurable_and_leave_editors_and_search_alone() {
    let map = ShortcutMap::new(&[]).unwrap();
    for (chord, expected) in [
        ("Alt+Down", "compare.next-difference"),
        ("Alt+Right", "compare.next-difference"),
        ("N", "compare.next-difference"),
        ("F7", "compare.next-difference"),
        ("Alt+Up", "compare.previous-difference"),
        ("Alt+Left", "compare.previous-difference"),
        ("P", "compare.previous-difference"),
        ("Shift+F7", "compare.previous-difference"),
        ("Alt+Home", "compare.first-difference"),
        ("Alt+End", "compare.last-difference"),
    ] {
        assert_eq!(
            action(&map, chord, ShortcutContext::Comparison).as_deref(),
            Some(expected)
        );
        assert_eq!(action(&map, chord, ShortcutContext::ComparisonSearch), None);
    }
    assert_eq!(
        action(&map, "F3", ShortcutContext::Comparison).as_deref(),
        Some("viewer.find-next")
    );
    assert_eq!(
        action(&map, "Down", ShortcutContext::Comparison).as_deref(),
        Some("viewer.down")
    );
    let map = ShortcutMap::new(&[
        ShortcutBinding {
            action: "compare.next-difference".into(),
            keys: vec!["Ctrl+J".into()],
        },
        ShortcutBinding {
            action: "compare.first-difference".into(),
            keys: vec![],
        },
    ])
    .unwrap();
    for chord in ["F7", "Alt+Down", "Alt+Right", "N", "Alt+Home"] {
        assert_eq!(action(&map, chord, ShortcutContext::Comparison), None);
    }
    assert_eq!(
        action(&map, "Ctrl+J", ShortcutContext::Comparison).as_deref(),
        Some("compare.next-difference")
    );
}

#[test]
fn control_f_finds_files_or_viewer_content_in_the_appropriate_context() {
    let map = ShortcutMap::new(&[]).unwrap();
    for context in [ShortcutContext::Files, ShortcutContext::Search] {
        assert_eq!(
            action(&map, "Ctrl+F", context).as_deref(),
            Some("files.find")
        );
        assert_eq!(
            action(&map, "Alt+F7", context).as_deref(),
            Some("files.find")
        );
    }
    assert_eq!(
        action(&map, "Ctrl+F", ShortcutContext::Viewer).as_deref(),
        Some("viewer.find")
    );
    assert_eq!(action(&map, "Ctrl+F", ShortcutContext::Command), None);
    assert_eq!(action(&map, "Ctrl+F", ShortcutContext::Other), None);

    let reassigned = ShortcutMap::new(&[ShortcutBinding {
        action: "files.rename".into(),
        keys: vec!["Ctrl+F".into()],
    }])
    .unwrap();
    assert_eq!(
        action(&reassigned, "Ctrl+F", ShortcutContext::Files).as_deref(),
        Some("files.rename")
    );
    assert_eq!(
        action(&reassigned, "Ctrl+F", ShortcutContext::Viewer).as_deref(),
        Some("viewer.find")
    );
    let disabled = ShortcutMap::new(&[ShortcutBinding {
        action: "files.find".into(),
        keys: vec![],
    }])
    .unwrap();
    assert_eq!(action(&disabled, "Ctrl+F", ShortcutContext::Files), None);
}

#[test]
fn bare_hot_path_digits_are_configurable_and_never_steal_editor_text() {
    let map = ShortcutMap::new(&[]).unwrap();
    for (key, id) in [("1", "bookmark.open.1"), ("0", "bookmark.open.10")] {
        assert_eq!(
            action(&map, key, ShortcutContext::Files).as_deref(),
            Some(id)
        );
        for context in [
            ShortcutContext::Search,
            ShortcutContext::Command,
            ShortcutContext::Other,
        ] {
            assert_eq!(action(&map, key, context), None);
        }
    }
    let map = ShortcutMap::new(&[ShortcutBinding {
        action: "bookmark.open.1".into(),
        keys: vec!["Ctrl+Q".into()],
    }])
    .unwrap();
    assert_eq!(action(&map, "1", ShortcutContext::Files), None);
    assert_eq!(
        action(&map, "Ctrl+Q", ShortcutContext::Files).as_deref(),
        Some("bookmark.open.1")
    );
}

#[test]
fn explicit_digit_bindings_override_only_new_defaults_in_overlapping_contexts() {
    let map = ShortcutMap::new(&[ShortcutBinding {
        action: "files.rename".into(),
        keys: vec!["1".into()],
    }])
    .unwrap();
    assert_eq!(
        action(&map, "1", ShortcutContext::Files).as_deref(),
        Some("files.rename")
    );
    assert_eq!(
        action(&map, "Ctrl+1", ShortcutContext::Files).as_deref(),
        Some("bookmark.open.1")
    );
    assert_eq!(
        action(&ShortcutMap::new(&[]).unwrap(), "1", ShortcutContext::Files).as_deref(),
        Some("bookmark.open.1")
    );
    assert!(
        ShortcutMap::new(&[
            ShortcutBinding {
                action: "files.rename".into(),
                keys: vec!["1".into()]
            },
            ShortcutBinding {
                action: "bookmark.open.1".into(),
                keys: vec!["1".into()]
            }
        ])
        .is_err(),
        "two explicit assignments must still conflict"
    );
    let map = ShortcutMap::new(&[ShortcutBinding {
        action: "viewer.next-file".into(),
        keys: vec!["1".into()],
    }])
    .unwrap();
    assert_eq!(
        action(&map, "1", ShortcutContext::Files).as_deref(),
        Some("bookmark.open.1")
    );
    assert_eq!(
        action(&map, "1", ShortcutContext::Viewer).as_deref(),
        Some("viewer.next-file")
    );
}

#[test]
fn commander_tools_and_history_have_configurable_context_bound_defaults() {
    let map = ShortcutMap::new(&[]).unwrap();
    for (key, id) in [
        ("Ctrl+F2", "files.attributes"),
        ("Ctrl+F7", "files.case"),
        ("Ctrl+F10", "panel.compare"),
        ("Ctrl+F11", "panel.zoom"),
        ("Ctrl+F12", "panel.filter"),
    ] {
        assert_eq!(
            action(&map, key, ShortcutContext::Files).as_deref(),
            Some(id)
        );
        assert_eq!(
            action(&map, key, ShortcutContext::Search).as_deref(),
            Some(id)
        );
        assert_eq!(action(&map, key, ShortcutContext::Command), None);
        let overrides = [ShortcutBinding {
            action: id.into(),
            keys: vec![],
        }];
        assert_eq!(
            action(
                &ShortcutMap::new(&overrides).unwrap(),
                key,
                ShortcutContext::Files
            ),
            None
        );
    }
    for (key, history, selected) in [
        ("Alt+Up", "command.history-older", "panel.previous-selected"),
        ("Alt+Down", "command.history-newer", "panel.next-selected"),
    ] {
        assert_eq!(
            action(&map, key, ShortcutContext::Command).as_deref(),
            Some(history)
        );
        assert_eq!(
            action(&map, key, ShortcutContext::Files).as_deref(),
            Some(selected)
        );
        assert_eq!(action(&map, key, ShortcutContext::Search), None);
        assert_eq!(action(&map, key, ShortcutContext::Other), None);
    }
}

fn action(map: &ShortcutMap, keys: &str, context: ShortcutContext) -> Option<String> {
    map.resolve(&ShortcutChord::parse(keys).unwrap(), context)
        .map(|action| action.id.clone())
}

#[test]
fn introducing_f5_copy_keeps_previously_saved_f5_assignments_valid() {
    for (id, context) in [
        ("files.rename", ShortcutContext::Files),
        ("search.clear", ShortcutContext::Search),
        ("app.help", ShortcutContext::Other),
    ] {
        let binding = ShortcutBinding {
            action: id.into(),
            keys: vec!["F5".into()],
        };
        let map = ShortcutMap::new(std::slice::from_ref(&binding)).unwrap();
        assert_eq!(action(&map, "F5", context).as_deref(), Some(id));
        assert!(
            map.bindings("files.copy-to").unwrap().is_empty(),
            "do not advertise a shadowed F5 default"
        );
        assert!(
            ShortcutMap::new(&[
                binding,
                ShortcutBinding {
                    action: "files.copy-to".into(),
                    keys: vec!["F5".into()]
                },
            ])
            .is_err(),
            "explicit assignments must still report conflicts"
        );
    }
    let map = ShortcutMap::new(&[ShortcutBinding {
        action: "viewer.next-file".into(),
        keys: vec!["F5".into()],
    }])
    .unwrap();
    assert_eq!(
        action(&map, "F5", ShortcutContext::Viewer).as_deref(),
        Some("viewer.next-file")
    );
    assert_eq!(
        action(&map, "F5", ShortcutContext::Files).as_deref(),
        Some("files.copy-to")
    );
}

#[test]
fn all_default_actions_are_named_unique_and_resolve_without_context_collisions() {
    let map = ShortcutMap::new(&[]).unwrap();
    let mut names = std::collections::HashSet::new();
    for definition in shortcut_catalog() {
        assert!(names.insert(&definition.id));
        assert!(!definition.label.is_empty());
        for keys in &definition.defaults {
            assert!(ShortcutChord::parse(keys).is_ok());
        }
    }
    assert_eq!(
        action(&map, "Ctrl+C", ShortcutContext::Files),
        Some("files.copy".into())
    );
    assert_eq!(
        action(&map, "Ctrl+V", ShortcutContext::Files),
        Some("files.paste".into())
    );
    assert_eq!(action(&map, "Ctrl+C", ShortcutContext::Command), None);
    assert_eq!(
        action(&map, "Ctrl+Enter", ShortcutContext::Files),
        Some("command.insert-name".into())
    );
    assert_eq!(
        action(&map, "Backspace", ShortcutContext::Files),
        Some("panel.parent".into())
    );
    assert_eq!(
        action(&map, "Backspace", ShortcutContext::Search),
        Some("search.erase".into())
    );
    assert_eq!(
        action(&map, "Ctrl+Shift+Digit3", ShortcutContext::Files),
        Some("tab.3".into())
    );
    assert_eq!(action(&map, "Shift+Digit3", ShortcutContext::Search), None);
}

#[test]
fn overrides_replace_all_aliases_and_empty_binding_disables_the_action() {
    let map = ShortcutMap::new(&[
        ShortcutBinding {
            action: "files.rename".into(),
            keys: vec!["Ctrl+Q".into()],
        },
        ShortcutBinding {
            action: "files.trash".into(),
            keys: vec![],
        },
    ])
    .unwrap();
    assert_eq!(
        action(&map, "Ctrl+Q", ShortcutContext::Files),
        Some("files.rename".into())
    );
    for old in ["F2", "F8", "Delete"] {
        assert_eq!(action(&map, old, ShortcutContext::Files), None);
    }
    assert_eq!(action(&map, "Ctrl+Q", ShortcutContext::Command), None);
    assert_eq!(action(&map, "Ctrl+Q", ShortcutContext::Other), None);
}

#[test]
fn conflicts_bad_syntax_and_unknown_actions_fail_without_partially_applying() {
    for bindings in [
        vec![ShortcutBinding {
            action: "files.rename".into(),
            keys: vec!["Ctrl+C".into()],
        }],
        vec![ShortcutBinding {
            action: "files.rename".into(),
            keys: vec!["F1".into()],
        }],
        vec![ShortcutBinding {
            action: "unknown.future.command".into(),
            keys: vec!["Ctrl+Q".into()],
        }],
        vec![ShortcutBinding {
            action: "files.rename".into(),
            keys: vec!["Ctrl+Ctrl+Q".into()],
        }],
        vec![ShortcutBinding {
            action: "files.rename".into(),
            keys: vec!["Ctrl+Q".into(), "CTRL+q".into()],
        }],
    ] {
        assert!(ShortcutMap::new(&bindings).is_err(), "{bindings:?}");
    }
    for invalid in [
        "",
        "Ctrl",
        "Ctrl+",
        "F99",
        "Ctrl+two keys",
        "Ctrl+\0",
        "Ctrl+Alt+Shift+Meta+Extra",
    ] {
        assert!(ShortcutChord::parse(invalid).is_err(), "{invalid:?}");
    }
    assert_eq!(
        ShortcutChord::parse("control + shift + return")
            .unwrap()
            .display(),
        "Ctrl+Shift+Enter"
    );
    assert_eq!(
        ShortcutChord::parse("Ctrl+Plus").unwrap().display(),
        "Ctrl+Plus"
    );
}

#[test]
fn parser_preserves_unicode_and_canonicalizes_physical_digit_aliases() {
    for (input, canonical) in [
        ("alt + CTRL + q", "Ctrl+Alt+Q"),
        ("Ctrl+arrowleft", "Ctrl+Left"),
        ("Meta+文", "Meta+文"),
        ("Ctrl+é", "Ctrl+é"),
        ("Ctrl+É", "Ctrl+É"),
        ("shift+digit0", "Shift+Digit0"),
        ("Ctrl+[", "Ctrl+BracketLeft"),
    ] {
        let chord = ShortcutChord::parse(input).unwrap();
        assert_eq!(chord.display(), canonical);
        assert_eq!(ShortcutChord::parse(&chord.display()).unwrap(), chord);
    }
    assert_eq!(
        ShortcutChord::parse("Ctrl+1").unwrap(),
        ShortcutChord::parse("Ctrl+Digit1").unwrap()
    );
    assert!(
        ShortcutMap::new(&[ShortcutBinding {
            action: "files.rename".into(),
            keys: vec!["Ctrl+1".into()],
        }])
        .is_err(),
        "literal digit alias must conflict with the existing physical Hot Path"
    );
    for invalid in [
        "Ctrl+Control+Q",
        "Alt+alt+Q",
        "Q+Ctrl",
        "Ctrl++",
        "Ctrl+F01",
        "Shift+Digit10",
    ] {
        assert!(ShortcutChord::parse(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn bindings_can_share_keys_only_in_disjoint_contexts_and_report_effective_hints() {
    let bindings = [
        ShortcutBinding {
            action: "files.rename".into(),
            keys: vec!["Ctrl+Q".into()],
        },
        ShortcutBinding {
            action: "command.execute".into(),
            keys: vec!["Ctrl+Q".into()],
        },
    ];
    let map = ShortcutMap::new(&bindings).unwrap();
    assert_eq!(
        action(&map, "Ctrl+Q", ShortcutContext::Files),
        Some("files.rename".into())
    );
    assert_eq!(
        action(&map, "Ctrl+Q", ShortcutContext::Command),
        Some("command.execute".into())
    );
    assert_eq!(map.bindings("files.rename").unwrap()[0].display(), "Ctrl+Q");
    assert!(map.bindings("missing").is_none());
    let mut conflict = bindings.to_vec();
    conflict.push(ShortcutBinding {
        action: "app.help".into(),
        keys: vec!["Ctrl+Q".into()],
    });
    let error = ShortcutMap::new(&conflict).unwrap_err();
    assert!(error.contains("Ctrl+Q") && error.contains("Rename") && error.contains("Help"));
}

#[test]
fn shortcut_resource_limits_and_duplicate_override_actions_are_rejected() {
    let binding = ShortcutBinding {
        action: "files.rename".into(),
        keys: vec![],
    };
    assert!(ShortcutMap::new(&vec![binding.clone(); MAX_SHORTCUT_OVERRIDES + 1]).is_err());
    assert!(ShortcutMap::new(&[binding.clone(), binding]).is_err());
    assert!(
        ShortcutMap::new(&[ShortcutBinding {
            action: "files.rename".into(),
            keys: vec!["Ctrl+Q".into(); MAX_SHORTCUT_KEYS + 1]
        }])
        .is_err()
    );
    assert!(ShortcutChord::parse(&"x".repeat(MAX_SHORTCUT_CHORD_BYTES + 1)).is_err());
    let map = ShortcutMap::new(&[]).unwrap();
    assert_eq!(
        action(&map, "Ctrl+PageUp", ShortcutContext::Search),
        Some("panel.parent".into())
    );
    assert_eq!(
        action(&map, "Ctrl+Digit3", ShortcutContext::Search),
        Some("bookmark.open.3".into())
    );
    assert_eq!(
        action(&map, "Shift+Digit3", ShortcutContext::Files),
        Some("bookmark.open.3".into())
    );
    assert_eq!(action(&map, "Ctrl+Space", ShortcutContext::Files), None);
}

#[test]
fn move_and_cut_defaults_preserve_existing_explicit_clipboard_assignments() {
    let defaults = ShortcutMap::new(&[]).unwrap();
    assert_eq!(
        action(&defaults, "F6", ShortcutContext::Files),
        Some("files.move-to".into())
    );
    assert_eq!(
        action(&defaults, "Ctrl+X", ShortcutContext::Files),
        Some("files.cut".into())
    );
    assert_eq!(action(&defaults, "Ctrl+X", ShortcutContext::Search), None);
    let map = ShortcutMap::new(&[ShortcutBinding {
        action: "files.copy".into(),
        keys: vec!["F6".into(), "Ctrl+X".into()],
    }])
    .unwrap();
    assert_eq!(
        action(&map, "F6", ShortcutContext::Files),
        Some("files.copy".into())
    );
    assert_eq!(
        action(&map, "Ctrl+X", ShortcutContext::Files),
        Some("files.copy".into())
    );
    assert!(map.bindings("files.move-to").unwrap().is_empty());
    assert!(map.bindings("files.cut").unwrap().is_empty());
}

#[test]
fn native_editing_and_ime_keys_cannot_be_stolen_by_application_shortcuts() {
    for reserved in [
        "Ctrl+Space",
        "Ctrl+C",
        "Ctrl+X",
        "Ctrl+A",
        "Ctrl+V",
        "Ctrl+Z",
        "Ctrl+Shift+Z",
        "Ctrl+Y",
        "Ctrl+Insert",
        "Shift+Insert",
        "Shift+Delete",
        "Ctrl+Backspace",
        "Ctrl+Delete",
        "Ctrl+Left",
        "Ctrl+Shift+Right",
        "Ctrl+Home",
        "Ctrl+End",
        "Q",
        "Shift+Q",
        "Digit3",
        "Space",
        "Backspace",
        "Tab",
        "Shift+Tab",
        "Left",
        "Enter",
        "Escape",
    ] {
        let bindings = [
            ShortcutBinding {
                action: "files.copy".into(),
                keys: vec![],
            },
            ShortcutBinding {
                action: "files.paste".into(),
                keys: vec![],
            },
            ShortcutBinding {
                action: "selection.all".into(),
                keys: vec![],
            },
            ShortcutBinding {
                action: "files.delete".into(),
                keys: vec![],
            },
            ShortcutBinding {
                action: "panel.switch".into(),
                keys: vec![],
            },
            ShortcutBinding {
                action: "app.help".into(),
                keys: vec![reserved.into()],
            },
        ];
        assert!(
            ShortcutMap::new(&bindings).is_err(),
            "global action accepted {reserved}"
        );
    }
    for reserved in [
        "Ctrl+Space",
        "Q",
        "Shift+Q",
        "Ctrl+C",
        "Ctrl+Left",
        "Backspace",
        "Tab",
    ] {
        assert!(
            ShortcutMap::new(&[ShortcutBinding {
                action: "command.execute".into(),
                keys: vec![reserved.into()]
            }])
            .is_err(),
            "command action accepted {reserved}"
        );
    }
    assert!(
        ShortcutMap::new(&[ShortcutBinding {
            action: "files.rename".into(),
            keys: vec!["Ctrl+Space".into()]
        }])
        .is_err()
    );
    assert!(
        ShortcutMap::new(&[
            ShortcutBinding {
                action: "files.copy".into(),
                keys: vec![]
            },
            ShortcutBinding {
                action: "files.rename".into(),
                keys: vec!["Ctrl+C".into()]
            },
        ])
        .is_ok()
    );
}

#[test]
fn volume_popup_mnemonic_is_configurable_without_intercepting_editor_punctuation() {
    let map = ShortcutMap::new(&[]).unwrap();
    assert_eq!(
        action(&map, ".", ShortcutContext::Volume),
        Some("panel.as-other".into())
    );
    for context in [
        ShortcutContext::Files,
        ShortcutContext::Search,
        ShortcutContext::Command,
        ShortcutContext::Other,
    ] {
        assert_eq!(action(&map, ".", context), None);
    }
    let map = ShortcutMap::new(&[ShortcutBinding {
        action: "panel.as-other".into(),
        keys: vec!["Ctrl+Q".into()],
    }])
    .unwrap();
    assert_eq!(
        action(&map, "Ctrl+Q", ShortcutContext::Volume),
        Some("panel.as-other".into())
    );
    assert_eq!(action(&map, ".", ShortcutContext::Volume), None);
    for key in ["F1", "Enter", "Escape", "Up", "Down", "Tab"] {
        assert!(
            ShortcutMap::new(&[ShortcutBinding {
                action: "panel.as-other".into(),
                keys: vec![key.into()]
            }])
            .is_err(),
            "popup accepted reserved or conflicting key {key}"
        );
    }
    let disabled = ShortcutMap::new(&[ShortcutBinding {
        action: "panel.as-other".into(),
        keys: vec![],
    }])
    .unwrap();
    assert_eq!(action(&disabled, ".", ShortcutContext::Volume), None);
}

#[test]
fn reassigned_file_actions_preserve_clipboard_keys_in_filename_search() {
    for keys in [
        "Ctrl+C",
        "Ctrl+X",
        "Ctrl+V",
        "Meta+C",
        "Meta+X",
        "Meta+V",
        "Ctrl+Insert",
        "Shift+Insert",
    ] {
        let map = ShortcutMap::new(&[
            ShortcutBinding {
                action: "files.copy".into(),
                keys: vec![],
            },
            ShortcutBinding {
                action: "files.paste".into(),
                keys: vec![],
            },
            ShortcutBinding {
                action: "files.rename".into(),
                keys: vec![keys.into()],
            },
        ])
        .unwrap();
        assert_eq!(
            action(&map, keys, ShortcutContext::Files),
            Some("files.rename".into())
        );
        assert_eq!(
            action(&map, keys, ShortcutContext::Search),
            None,
            "{keys} must remain available to the filename search editor"
        );
        assert!(
            ShortcutMap::new(&[
                ShortcutBinding {
                    action: "files.copy".into(),
                    keys: vec![]
                },
                ShortcutBinding {
                    action: "files.paste".into(),
                    keys: vec![]
                },
                ShortcutBinding {
                    action: "search.clear".into(),
                    keys: vec![keys.into()]
                },
            ])
            .is_err(),
            "a search-only action must not claim {keys}"
        );
    }
    let map = ShortcutMap::new(&[]).unwrap();
    assert_eq!(
        action(&map, "Ctrl+A", ShortcutContext::Search),
        Some("selection.files".into())
    );
    assert_eq!(
        action(&map, "Shift+Delete", ShortcutContext::Search),
        Some("files.delete".into())
    );
}
#[test]
fn make_file_list_uses_configurable_control_m_without_stealing_editor_input() {
    let map = ShortcutMap::new(&[]).unwrap();
    assert_eq!(
        action(&map, "Ctrl+M", ShortcutContext::Files).as_deref(),
        Some("files.make-list")
    );
    assert_eq!(action(&map, "Ctrl+M", ShortcutContext::Command), None);
    assert_eq!(action(&map, "Ctrl+M", ShortcutContext::Viewer), None);
    let custom = ShortcutMap::new(&[ShortcutBinding {
        action: "files.make-list".into(),
        keys: vec!["Ctrl+Shift+M".into()],
    }])
    .unwrap();
    assert_eq!(action(&custom, "Ctrl+M", ShortcutContext::Files), None);
    assert_eq!(
        action(&custom, "Ctrl+Shift+M", ShortcutContext::Files).as_deref(),
        Some("files.make-list")
    );
}

#[test]
fn changed_selection_defaults_preserve_explicit_legacy_assignments() {
    for (key, saved_action, default_action) in [
        ("Ctrl+A", "selection.all", "selection.files"),
        ("Ctrl+D", "files.rename", "selection.clear"),
    ] {
        let saved = ShortcutBinding {
            action: saved_action.into(),
            keys: vec![key.into()],
        };
        let map = ShortcutMap::new(std::slice::from_ref(&saved)).unwrap();
        assert_eq!(
            action(&map, key, ShortcutContext::Files).as_deref(),
            Some(saved_action)
        );
        assert!(
            map.bindings(default_action).unwrap().is_empty(),
            "{default_action} must not advertise the suppressed {key}"
        );
        assert!(
            ShortcutMap::new(&[
                saved,
                ShortcutBinding {
                    action: default_action.into(),
                    keys: vec![key.into()]
                }
            ])
            .is_err()
        );
    }
    let map = ShortcutMap::new(&[ShortcutBinding {
        action: "selection.clear-files".into(),
        keys: vec![],
    }])
    .unwrap();
    assert_eq!(action(&map, "Ctrl+Shift+D", ShortcutContext::Files), None);
    assert_eq!(
        action(&map, "Ctrl+D", ShortcutContext::Files).as_deref(),
        Some("selection.clear")
    );
}

#[test]
fn clear_files_only_never_changes_directory_selection() {
    for selected in [false, true] {
        assert_eq!(
            SelectionOperation::ClearFilesOnly.selects(true, selected),
            selected
        );
        assert!(!SelectionOperation::ClearFilesOnly.selects(false, selected));
        assert!(!SelectionOperation::Clear.selects(true, selected));
        assert!(!SelectionOperation::Clear.selects(false, selected));
    }
}

const PANEL_VIEW_KEYS: [(&str, &str); 3] = [
    ("view.brief", "Alt+Digit2"),
    ("view.detailed", "Alt+Digit3"),
    ("view.thumbnails", "Alt+Digit5"),
];

#[test]
fn panel_view_defaults_work_in_files_and_search_without_stealing_editor_keys() {
    let map = ShortcutMap::new(&[]).unwrap();
    for (id, key) in PANEL_VIEW_KEYS {
        for context in [ShortcutContext::Files, ShortcutContext::Search] {
            assert_eq!(action(&map, key, context).as_deref(), Some(id), "{key}");
        }
        for context in [
            ShortcutContext::Command,
            ShortcutContext::Other,
            ShortcutContext::Viewer,
        ] {
            assert_eq!(action(&map, key, context), None, "{key} in {context:?}");
        }
        assert_eq!(map.bindings(id).unwrap()[0].display(), key);
    }
}

#[test]
fn panel_view_defaults_preserve_saved_assignments_and_support_remapping_or_removal() {
    for (id, key) in PANEL_VIEW_KEYS {
        let saved = ShortcutBinding {
            action: "files.rename".into(),
            keys: vec![key.into()],
        };
        let map = ShortcutMap::new(std::slice::from_ref(&saved)).unwrap();
        assert_eq!(
            action(&map, key, ShortcutContext::Files).as_deref(),
            Some("files.rename")
        );
        assert!(map.bindings(id).unwrap().is_empty());
        assert!(
            ShortcutMap::new(&[
                saved,
                ShortcutBinding {
                    action: id.into(),
                    keys: vec![key.into()]
                },
            ])
            .is_err(),
            "explicit conflicts must still be rejected"
        );

        let remapped = ShortcutMap::new(&[ShortcutBinding {
            action: id.into(),
            keys: vec!["Ctrl+Shift+F12".into()],
        }])
        .unwrap();
        assert_eq!(action(&remapped, key, ShortcutContext::Files), None);
        assert_eq!(
            action(&remapped, "Ctrl+Shift+F12", ShortcutContext::Files).as_deref(),
            Some(id)
        );
        let removed = ShortcutMap::new(&[ShortcutBinding {
            action: id.into(),
            keys: vec![],
        }])
        .unwrap();
        assert_eq!(action(&removed, key, ShortcutContext::Files), None);
        assert!(removed.bindings(id).unwrap().is_empty());
    }
}

#[test]
fn panel_view_previous_defaults_only_remain_when_explicitly_saved() {
    let defaults = ShortcutMap::new(&[]).unwrap();
    for ((id, salamander), previous) in
        PANEL_VIEW_KEYS
            .into_iter()
            .zip(["Ctrl+Shift+B", "Ctrl+Shift+L", "Ctrl+Shift+T"])
    {
        assert_eq!(action(&defaults, previous, ShortcutContext::Files), None);
        let saved = ShortcutMap::new(&[ShortcutBinding {
            action: id.into(),
            keys: vec![previous.into()],
        }])
        .unwrap();
        assert_eq!(
            action(&saved, previous, ShortcutContext::Files).as_deref(),
            Some(id)
        );
        assert_eq!(action(&saved, salamander, ShortcutContext::Files), None);
    }
}
