use super::*;

fn binding(action: &str, keys: &[&str]) -> ShortcutBinding {
    ShortcutBinding {
        action: action.into(),
        keys: keys.iter().map(|key| (*key).into()).collect(),
    }
}

#[test]
fn keyboard_changes_validate_before_replacing_aliases_or_saving() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.apply(Event::PreferencesLoaded(Ok(Box::default())));
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    assert!(p.change_shortcuts("files.copy", vec!["Ctrl+Alt+Y".into()], &send));
    assert_eq!(
        p.preferences.values.shortcuts,
        vec![binding("files.copy", &["Ctrl+Alt+Y"])]
    );
    let revision = p.preferences.revision;
    let snapshot = p.keyboard.bindings.borrow().clone();
    assert_eq!(
        snapshot.map.resolve(
            &ShortcutChord::parse("Ctrl+C").unwrap(),
            ShortcutContext::Files
        ),
        None
    );
    assert_eq!(
        snapshot
            .map
            .resolve(
                &ShortcutChord::parse("Ctrl+Alt+Y").unwrap(),
                ShortcutContext::Files
            )
            .unwrap()
            .id,
        "files.copy"
    );
    sent.borrow_mut().clear();
    assert!(!p.change_shortcuts("files.copy", vec!["Ctrl+V".into()], &send));
    assert!(!p.keyboard.error.is_empty());
    assert!(sent.borrow().is_empty());
    assert_eq!(p.preferences.revision, revision);
    assert!(Arc::ptr_eq(&snapshot, &p.keyboard.bindings.borrow()));
    assert!(!p.change_shortcuts("app.help", vec!["Ctrl+C".into()], &send));
    assert!(p.change_shortcuts("files.copy", vec![], &send));
    assert!(
        p.keyboard
            .bindings
            .borrow()
            .map
            .bindings("files.copy")
            .unwrap()
            .is_empty()
    );
    assert!(p.restore_shortcut("files.copy", &send));
    assert!(p.preferences.values.shortcuts.is_empty());
    assert_eq!(
        p.keyboard
            .bindings
            .borrow()
            .map
            .bindings("files.copy")
            .unwrap()[0]
            .display(),
        "Ctrl+C"
    );
}

#[test]
fn shortcut_cache_survives_unrelated_preferences_and_early_edits_survive_initial_load() {
    let mut p = Presenter::new("left".into(), "right".into());
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    assert!(p.change_shortcuts("files.copy", vec!["Ctrl+Alt+Y".into()], &send));
    assert!(sent.borrow().is_empty(), "initial read must precede save");
    let snapshot = p.keyboard.bindings.borrow().clone();
    p.change_preferences(1, 0, 0, &send);
    assert!(
        Arc::ptr_eq(&snapshot, &p.keyboard.bindings.borrow()),
        "theme changes must not recompile keys"
    );
    p.apply_with_sender(
        Event::PreferencesLoaded(Ok(Box::new(UserPreferences {
            shortcuts: vec![binding("files.copy", &["Ctrl+Alt+K"])],
            ..Default::default()
        }))),
        &send,
    );
    assert_eq!(
        p.preferences.values.shortcuts,
        vec![binding("files.copy", &["Ctrl+Alt+Y"])]
    );
    assert!(
        sent.borrow()
            .iter()
            .any(|command| matches!(command, Command::SavePreferences { .. }))
    );
    p.restore_shortcuts(&send);
    assert!(p.preferences.values.shortcuts.is_empty());
    let snapshot = p.keyboard.bindings.borrow().clone();
    let editor_revision = p.keyboard.editor_revision_text.clone();
    assert!(p.restore_shortcut("files.copy", &send));
    assert!(Arc::ptr_eq(&snapshot, &p.keyboard.bindings.borrow()));
    assert_ne!(p.keyboard.editor_revision_text, editor_revision);
}

#[test]
fn shortcut_scope_explains_each_alias_when_native_search_rules_differ() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.apply(Event::PreferencesLoaded(Ok(Box::default())));
    assert!(p.change_shortcuts("files.copy", vec![], &|_| true));
    assert!(
        p.change_shortcuts("files.rename", vec!["Ctrl+C".into(), "F2".into()], &|_| {
            true
        },)
    );
    let snapshot = &p.keyboard.snapshot;
    let scope = snapshot.scope("files.rename");
    assert!(scope.contains("Ctrl+C — File list outside filename search"));
    assert!(scope.contains("F2 — File list and filename search"));
    assert!(snapshot.help.contains(scope));
}
