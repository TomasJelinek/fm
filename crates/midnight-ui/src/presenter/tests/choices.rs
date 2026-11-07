use super::*;

#[test]
fn one_drive_accounts_remain_in_menu_without_repeated_toolbar_buttons() {
    let root = test_root();
    let mut p = Presenter::new(root.clone(), root.clone());
    p.apply(Event::DesktopContext {
        shell: ShellFlavor::Posix,
        locations: vec![
            location("home", LocationKind::Home, root.clone()),
            location("personal", LocationKind::OneDrive, root.join("Personal")),
            location("business", LocationKind::OneDrive, root.join("Business")),
            location("temp", LocationKind::Temporary, root.join("Temp")),
        ],
    });
    let menu = crate::navigation_models::choices(&p);
    let toolbar = crate::navigation_models::toolbar_choices(&p, &menu);
    assert_eq!(menu.len(), 4);
    assert_eq!(toolbar.len(), 3);
    assert!(toolbar.iter().any(|row| row.identity == menu[1].identity));
    assert!(!toolbar.iter().any(|row| row.identity == menu[2].identity));
    assert!(menu[1].label.contains("personal") && menu[2].label.contains("business"));
    let selected = crate::navigation_models::locations(&toolbar, Some(&menu[3].identity));
    assert!(
        selected[2].selected,
        "filtering toolbar entries must not shift selection"
    );
}

fn location(id: &str, kind: LocationKind, path: PathBuf) -> NavigationLocation {
    NavigationLocation {
        id: id.into(),
        label: format!("{id} 文档 & [plain]"),
        path,
        kind,
        drive_letter: None,
    }
}

#[test]
fn canonical_choices_cover_every_volume_and_preserve_same_path_semantic_aliases() {
    let root = test_root();
    let home = root.join("Home 👩🏽‍💻");
    let mut p = Presenter::new(root.clone(), root.clone());
    p.apply(Event::VolumesUpdated(vec![
        Volume {
            id: "disk".into(),
            label: "Drive".into(),
            root: root.clone(),
        },
        Volume {
            id: "home".into(),
            label: "Home".into(),
            root: home.clone(),
        },
    ]));
    p.apply(Event::DesktopContext {
        shell: ShellFlavor::Posix,
        locations: vec![
            location("disk", LocationKind::Volume, root.clone()),
            location("home", LocationKind::Home, home.clone()),
            location("desktop", LocationKind::Desktop, home.clone()),
            location("documents", LocationKind::Documents, home.join("Documents")),
            location("downloads", LocationKind::Downloads, home.join("Downloads")),
        ],
    });
    assert_eq!(p.navigation_choices().len(), 5);
    assert_eq!(p.navigation_choices()[1].location.kind, LocationKind::Home);
    assert_eq!(
        p.navigation_choices()[2].location.kind,
        LocationKind::Desktop
    );
    assert_eq!(p.navigation_choices()[1].location.path, home);
    assert_eq!(p.navigation_choices()[2].location.path, home);
    assert_ne!(p.navigation_choice_token(1), p.navigation_choice_token(2));
}

#[test]
fn an_arbitrary_mount_using_home_id_remains_a_drive() {
    let root = test_root();
    let home = root.join("users/home");
    let mount = root.join("mounted/home");
    let mut p = Presenter::new(root.clone(), root);
    p.apply(Event::DesktopContext {
        shell: ShellFlavor::Posix,
        locations: vec![location("home", LocationKind::Home, home)],
    });
    p.apply(Event::VolumesUpdated(vec![Volume {
        id: "home".into(),
        label: "Home".into(),
        root: mount.clone(),
    }]));
    assert_eq!(p.navigation_choices().len(), 2);
    assert_eq!(
        p.navigation_choices()[0].location.kind,
        LocationKind::Volume
    );
    assert_eq!(p.navigation_choices()[0].location.path, mount);
    assert_eq!(p.navigation_choices()[1].location.kind, LocationKind::Home);
}

#[test]
fn authoritative_volumes_replace_old_context_roots_and_invalidate_reused_identity() {
    let root = test_root();
    let old_root = root.join("old");
    let new_root = root.join("new");
    let mut p = Presenter::new(root.clone(), root);
    p.apply(Event::DesktopContext {
        shell: ShellFlavor::Posix,
        locations: vec![location("same", LocationKind::Volume, old_root.clone())],
    });
    assert_eq!(p.navigation_choices().len(), 1);
    let old_token = p.navigation_choice_token(0);
    let old_revision = p.navigation_choices_revision();
    p.apply(Event::VolumesUpdated(vec![Volume {
        id: "same".into(),
        label: "Same label".into(),
        root: new_root.clone(),
    }]));
    assert_eq!(p.navigation_choices()[0].location.path, new_root);
    assert_ne!(p.navigation_choices_revision(), old_revision);
    assert!(!p.choose_navigation_choice(PaneId::Left, &old_token, &|_| {
        panic!("a stale popup or pressed button cannot navigate")
    }));
    p.apply(Event::VolumesUpdated(vec![]));
    assert!(
        p.navigation_choices().is_empty(),
        "removed context drives cannot reappear"
    );
}

#[test]
fn choices_keep_metadata_tokens_and_resolve_raw_targets_with_volume_memory() {
    let root = test_root();
    let remembered = root.join("last ' & 文档");
    let special = root.join("Desktop e\u{301} 🧪");
    let mut p = Presenter::new(root.clone(), root.clone());
    p.apply(Event::VolumesUpdated(vec![Volume {
        id: "disk".into(),
        label: "Before".into(),
        root: root.clone(),
    }]));
    p.apply(Event::DesktopContext {
        shell: ShellFlavor::Posix,
        locations: vec![location("desktop", LocationKind::Desktop, special.clone())],
    });
    let drive_token = p.navigation_choice_token(0);
    let special_token = p.navigation_choice_token(1);
    p.apply(Event::VolumesUpdated(vec![Volume {
        id: "disk".into(),
        label: "After 文档".into(),
        root: root.clone(),
    }]));
    assert_eq!(p.navigation_choice_token(0), drive_token);
    assert_eq!(p.navigation_choice_token(1), special_token);
    p.panes[0].volume_paths.insert(root, remembered.clone());
    for (token, expected) in [(drive_token, remembered), (special_token, special)] {
        let sent = RefCell::new(Vec::new());
        assert!(
            p.choose_navigation_choice(PaneId::Left, &token, &|command| {
                sent.borrow_mut().push(command);
                true
            })
        );
        assert!(sent.borrow().iter().any(|command| matches!(command,
            Command::Navigate { path, .. } | Command::Refresh { path, .. } if path == &expected)));
    }
    for invalid in ["", "not-a-token", "0:0", "1:1:1", "-1:1"] {
        assert!(!p.choose_navigation_choice(PaneId::Right, invalid, &|_| {
            panic!("invalid tokens cannot dispatch commands")
        }));
    }
}

#[test]
fn reordered_equal_labels_reject_old_tokens_and_rejected_navigation_keeps_alias() {
    let root = test_root();
    let shared = root.join("shared");
    let mut p = Presenter::new(shared.clone(), root.clone());
    p.apply(Event::DesktopContext {
        shell: ShellFlavor::Posix,
        locations: vec![
            location("home", LocationKind::Home, shared.clone()),
            location("desktop", LocationKind::Desktop, shared.clone()),
        ],
    });
    let home = p.navigation_choice_token(0);
    let desktop = p.navigation_choice_token(1);
    assert!(p.choose_navigation_choice(PaneId::Left, &desktop, &|_| true));
    assert_eq!(p.navigation_choice_index(PaneId::Left), -1);
    assert!(!p.choose_navigation_choice(PaneId::Left, &home, &|_| false));
    assert_eq!(p.navigation_choice_index(PaneId::Left), -1);
    p.apply(Event::DesktopContext {
        shell: ShellFlavor::Posix,
        locations: vec![
            location("desktop", LocationKind::Desktop, shared.clone()),
            location("home", LocationKind::Home, shared),
        ],
    });
    assert!(!p.choose_navigation_choice(PaneId::Right, &desktop, &|_| panic!("stale")));
    assert_eq!(p.navigation_choice_index(PaneId::Left), -1);
    assert!(p.choose_navigation_choice(PaneId::Right, &p.navigation_choice_token(1), &|_| true));
}

#[test]
fn drive_shortcuts_only_use_typed_native_metadata_for_the_current_raw_root() {
    let root = test_root().join("old");
    let new_root = test_root().join("new");
    let mut p = Presenter::new(root.clone(), root.clone());
    let mut disk = location("disk", LocationKind::Volume, root.clone());
    disk.drive_letter = Some('Q');
    p.apply(Event::DesktopContext {
        shell: ShellFlavor::Posix,
        locations: vec![disk],
    });
    p.apply(Event::VolumesUpdated(vec![Volume {
        id: "disk".into(),
        label: "Q:".into(),
        root: new_root.clone(),
    }]));
    assert_eq!(p.navigation_choices()[0].location.drive_letter, None);
    p.drive_letter(PaneId::Left, "Q", &|_| panic!("old native context"));
    let mut disk = location("disk", LocationKind::Volume, new_root.clone());
    disk.drive_letter = Some('Q');
    p.apply(Event::DesktopContext {
        shell: ShellFlavor::Posix,
        locations: vec![disk],
    });
    let sent = RefCell::new(false);
    p.drive_letter(PaneId::Right, "Q", &|command| {
        assert!(matches!(command, Command::Navigate { path, .. } if path == new_root));
        *sent.borrow_mut() = true;
        true
    });
    assert!(*sent.borrow());
}
