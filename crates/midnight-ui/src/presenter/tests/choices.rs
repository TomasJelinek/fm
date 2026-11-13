use super::*;

#[test]
fn one_drive_accounts_remain_in_menu_without_repeated_toolbar_buttons() {
    let root = test_root();
    let mut p = Presenter::new(root.clone(), root.clone());
    p.apply(Event::DesktopContext {
        system_locations: Vec::new(),
        network_position: 0,
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

fn recycle_context(root: &Path, position: Option<usize>) -> Event {
    Event::DesktopContext {
        shell: ShellFlavor::Posix,
        network_position: 1,
        system_locations: position.map_or_else(Vec::new, |special_position| {
            vec![SystemLocationChoice {
                location: SystemLocation::RecycleBin,
                label: "System Trash 文档".into(),
                special_position,
            }]
        }),
        locations: vec![
            location("disk", LocationKind::Volume, root.to_owned()),
            location("home", LocationKind::Home, root.join("Home")),
            location("documents", LocationKind::Documents, root.join("Documents")),
            location("personal", LocationKind::OneDrive, root.join("Personal")),
            location("business", LocationKind::OneDrive, root.join("Business")),
            location("last", LocationKind::Temporary, root.join("Last")),
        ],
    }
}

#[test]
fn recycle_bin_has_one_backend_position_on_both_surfaces_without_consuming_native_rows() {
    let root = test_root();
    let mut p = Presenter::new(root.clone(), root.clone());
    let mut context = recycle_context(&root, Some(2));
    if let Event::DesktopContext {
        system_locations, ..
    } = &mut context
    {
        system_locations.push(system_locations[0].clone());
    }
    p.apply(context);
    let menu = crate::navigation_models::ordered_choices(&p);
    let toolbar = crate::navigation_models::toolbar_choices(&p, &menu);
    for rows in [&menu, &toolbar] {
        let recycle: Vec<_> = rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.kind == crate::LocationRole::RecycleBin)
            .collect();
        assert_eq!(recycle.len(), 1);
        assert_eq!(recycle[0].0, 4);
        assert_eq!(recycle[0].1.label, "System Trash 文档");
        assert_eq!(recycle[0].1.tooltip, "Open system Recycle Bin/Trash");
    }
    let recycle = menu[4].identity.clone();
    let expected = vec![
        p.navigation_choice_token(0),
        p.navigation_choice_token(1),
        p.network_choice_token(),
        p.navigation_choice_token(2),
        recycle.to_string(),
        p.navigation_choice_token(3),
        p.navigation_choice_token(5),
    ];
    assert_eq!(
        toolbar
            .iter()
            .map(|row| row.identity.to_string())
            .collect::<Vec<_>>(),
        expected
    );
    assert!(
        menu.iter()
            .any(|row| row.identity == p.navigation_choice_token(4))
    );
    let projected =
        crate::navigation_models::locations(&toolbar, Some(&p.navigation_choice_token(5)));
    assert!(
        projected.last().unwrap().selected,
        "last native mapping must survive both synthetic rows"
    );
    let recycle = projected
        .iter()
        .find(|row| row.kind == crate::LocationRole::RecycleBin)
        .unwrap();
    assert!(!recycle.is_drive && recycle.path.is_empty() && !recycle.selected);
}

#[test]
fn recycle_bin_tokens_expire_when_position_native_inventory_or_capability_changes() {
    let root = test_root();
    let mut p = Presenter::new(root.clone(), root.clone());
    let token = |p: &Presenter| {
        crate::navigation_models::ordered_choices(p)
            .into_iter()
            .find(|row| row.kind == crate::LocationRole::RecycleBin)
            .unwrap()
            .identity
            .to_string()
    };
    p.apply(recycle_context(&root, Some(2)));
    let first = token(&p);
    p.apply(recycle_context(&root, Some(0)));
    let reordered = token(&p);
    assert_ne!(first, reordered);
    assert!(!p.choose_system_location(&first, &|_| panic!("stale position")));
    p.apply(recycle_context(&root, None));
    assert!(!p.choose_system_location(&reordered, &|_| panic!("revoked capability")));
    assert!(
        !crate::navigation_models::ordered_choices(&p)
            .iter()
            .any(|row| row.kind == crate::LocationRole::RecycleBin)
    );
    p.apply(recycle_context(&root, Some(0)));
    assert!(!p.choose_system_location(&reordered, &|_| panic!(
        "re-added capability cannot revive a token"
    )));
    let current = token(&p);
    p.apply(Event::VolumesUpdated(vec![]));
    assert!(!p.choose_system_location(&current, &|_| panic!("changed native inventory")));
}

#[test]
fn recycle_bin_refused_send_and_typed_completion_never_change_panel_authority() {
    let root = test_root();
    let mut p = Presenter::new(root.join("left"), root.join("right"));
    p.apply(recycle_context(&root, Some(2)));
    let token = crate::navigation_models::ordered_choices(&p)
        .into_iter()
        .find(|row| row.kind == crate::LocationRole::RecycleBin)
        .unwrap()
        .identity
        .to_string();
    let before = [0, 1].map(|index| {
        (
            p.panes[index].display_path().to_owned(),
            p.panes[index].generation,
            p.panes[index].cursor,
            p.panes[index].selected.clone(),
        )
    });
    assert!(!p.choose_system_location(&token, &|_| false));
    let sent = RefCell::new(Vec::new());
    assert!(p.choose_system_location(&token, &|command| {
        sent.borrow_mut().push(command);
        true
    }));
    let request = match sent.borrow().as_slice() {
        [
            Command::OpenSystemLocation {
                request,
                location: SystemLocation::RecycleBin,
            },
        ] => *request,
        commands => panic!("only a typed system shell request is permitted: {commands:?}"),
    };
    let status = p.desktop.status.clone();
    p.apply(Event::DesktopActionFinished {
        request: DesktopRequestId(request.0 + 1),
        result: Ok(DesktopActionResult::SystemLocationOpened {
            location: SystemLocation::RecycleBin,
        }),
    });
    assert_eq!(p.desktop.status, status);
    p.apply(Event::DesktopActionFinished {
        request,
        result: Ok(DesktopActionResult::SystemLocationOpened {
            location: SystemLocation::RecycleBin,
        }),
    });
    assert_ne!(p.desktop.status, status);
    for index in [0, 1] {
        assert_eq!(p.panes[index].display_path(), before[index].0.as_path());
        assert_eq!(p.panes[index].generation, before[index].1);
        assert_eq!(p.panes[index].cursor, before[index].2);
        assert!(Arc::ptr_eq(&p.panes[index].selected, &before[index].3));
    }
    let completed = p.desktop.status.clone();
    p.apply(Event::DesktopActionFinished {
        request,
        result: Ok(DesktopActionResult::SystemLocationOpened {
            location: SystemLocation::RecycleBin,
        }),
    });
    assert_eq!(p.desktop.status, completed);
    assert!(
        !p.choose_navigation_choice(PaneId::Left, &token, &|_| panic!(
            "a system choice is not a filesystem location"
        ))
    );
}

#[test]
fn recycle_bin_discovery_failure_invalidates_its_cached_capability() {
    let root = test_root();
    let mut p = Presenter::new(root.clone(), root.clone());
    p.apply(recycle_context(&root, Some(0)));
    let token = crate::navigation_models::ordered_choices(&p)
        .into_iter()
        .find(|row| row.kind == crate::LocationRole::RecycleBin)
        .unwrap()
        .identity
        .to_string();
    p.apply(Event::DesktopContextFailed(FsError::from_io(
        "desktop context",
        &root,
        std::io::Error::from(std::io::ErrorKind::Unsupported),
    )));
    assert!(!p.choose_system_location(&token, &|_| panic!(
        "failed discovery has no current capability"
    )));
    assert!(
        !crate::navigation_models::ordered_choices(&p)
            .iter()
            .any(|row| row.kind == crate::LocationRole::RecycleBin)
    );
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
        system_locations: Vec::new(),
        network_position: 0,
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
        system_locations: Vec::new(),
        network_position: 0,
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
        system_locations: Vec::new(),
        network_position: 0,
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
        system_locations: Vec::new(),
        network_position: 0,
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
        system_locations: Vec::new(),
        network_position: 0,
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
        system_locations: Vec::new(),
        network_position: 0,
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
        system_locations: Vec::new(),
        network_position: 0,
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
        system_locations: Vec::new(),
        network_position: 0,
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

#[test]
fn backend_locale_order_reaches_both_surfaces_without_reordering_hot_path_slots() {
    let root = test_root();
    let mut p = Presenter::new(root.join("Zulu"), root.join("Alpha"));
    p.apply(Event::PreferencesLoaded(Ok(Box::default())));
    assert!(p.assign_bookmark_slot(PaneId::Left, 1, &|_| true));
    assert!(p.assign_bookmark_slot(PaneId::Right, 10, &|_| true));
    let bookmarks = crate::panel_models::bookmark_choices(&p);
    let ordered = ["h", "ch", "i"];
    p.apply(Event::VolumesUpdated(
        ordered
            .iter()
            .map(|label| Volume {
                id: format!("drive-{label}"),
                label: (*label).into(),
                root: root.join(label),
            })
            .collect(),
    ));
    p.apply(Event::DesktopContext {
        system_locations: Vec::new(),
        network_position: 0,
        shell: ShellFlavor::Posix,
        locations: ordered
            .iter()
            .map(|label| NavigationLocation {
                id: format!("special-{label}"),
                label: (*label).into(),
                path: root.join("same alias"),
                kind: LocationKind::Documents,
                drive_letter: None,
            })
            .collect(),
    });
    let labels: Vec<_> = p
        .navigation_choices()
        .iter()
        .map(|choice| choice.location.label.as_str())
        .collect();
    assert_eq!(labels, ["h", "ch", "i", "h", "ch", "i"]);
    let semantic_rows = |rows: &[crate::NavigationChoiceRow]| {
        rows.iter()
            .map(|row| {
                (
                    row.identity.clone(),
                    row.label.clone(),
                    row.compact_label.clone(),
                    row.tooltip.clone(),
                    row.kind,
                )
            })
            .collect::<Vec<_>>()
    };
    let menu = crate::navigation_models::ordered_choices(&p);
    let toolbar = crate::navigation_models::toolbar_choices(&p, &menu);
    assert_eq!(
        semantic_rows(&toolbar),
        semantic_rows(&menu),
        "both surfaces retain backend section order and identities"
    );
    let after = crate::panel_models::bookmark_choices(&p);
    assert_eq!(
        semantic_rows(&after),
        semantic_rows(&bookmarks),
        "native inventory never sorts or renumbers user hot paths"
    );
    assert!(after[0].label.starts_with("1  ") && after[0].label.contains("Zulu"));
    assert!(after[1].label.starts_with("0  ") && after[1].label.contains("Alpha"));
}
