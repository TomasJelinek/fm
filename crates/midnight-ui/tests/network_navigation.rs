//! Exercise Network through the installed menu, pointer and keyboard bridge.
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{accessibility::AccessibleStringProperty, items::AccessibleRole};
use midnight_core::*;
use midnight_ui::{Presenter, install, refresh};
use slint::{Model, platform::Key};
use std::{cell::RefCell, rc::Rc, sync::Arc};

fn address(id: u8) -> NetworkAddress {
    NetworkAddress {
        provider: "network.test".into(),
        opaque: Arc::from([id]),
        display: if id == 1 { "Network" } else { "Server 文档" }.into(),
    }
}

#[test]
fn network_follows_all_drives_before_special_locations_on_both_surfaces() {
    let mut test = ui::TestUi::new();
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut presenter = Presenter::new(root.clone(), root.clone());
    presenter.apply(Event::VolumesUpdated(
        [("disk-a", "Alpha"), ("disk-z", "Zulu"), ("home", "Home")]
            .into_iter()
            .map(|(id, label)| Volume {
                id: id.into(),
                label: label.into(),
                root: root.join(label),
            })
            .collect(),
    ));
    // Volume discovery may finish before desktop shortcuts. Its synthetic Home
    // still belongs after Network, which must not jump as discovery completes.
    refresh(&test.window, &presenter);
    let initial: Vec<_> = test.window.get_left_locations().iter().collect();
    assert_eq!(initial.len(), 4);
    assert_eq!(initial[0].kind, midnight_ui::LocationRole::Drive);
    assert_eq!(initial[1].kind, midnight_ui::LocationRole::Drive);
    assert_eq!(initial[2].kind, midnight_ui::LocationRole::Network);
    assert_eq!(initial[3].kind, midnight_ui::LocationRole::Home);
    presenter.apply(Event::DesktopContext {
        system_locations: Vec::new(),
        network_position: 0,
        shell: ShellFlavor::Posix,
        locations: [
            ("Home", LocationKind::Home),
            ("Music", LocationKind::Music),
            ("Pictures", LocationKind::Pictures),
        ]
        .into_iter()
        .map(|(label, kind)| NavigationLocation {
            id: label.to_ascii_lowercase(),
            label: label.into(),
            path: root.join(label),
            kind,
            drive_letter: None,
        })
        .collect(),
    });
    presenter.apply(Event::NetworkAvailability(true));
    refresh(&test.window, &presenter);
    test.draw();
    let dropdown: Vec<_> = test
        .window
        .get_navigation_choices()
        .iter()
        .filter(|row| row.identity != "other-panel")
        .map(|row| row.identity)
        .collect();
    let toolbar: Vec<_> = test
        .window
        .get_left_locations()
        .iter()
        .map(|row| row.identity)
        .collect();
    assert_eq!(
        toolbar, dropdown,
        "Network must be the same shared ordered location on both surfaces"
    );
    assert_eq!(
        toolbar,
        test.window
            .get_right_locations()
            .iter()
            .map(|row| row.identity)
            .collect::<Vec<_>>()
    );
    let rows: Vec<_> = test.window.get_left_locations().iter().collect();
    assert_eq!(rows.len(), 6);
    assert!(rows[0].label.starts_with("Alpha"));
    assert!(rows[1].label.starts_with("Zulu"));
    assert_eq!(rows[2].kind, midnight_ui::LocationRole::Network);
    assert!(rows[3].label.starts_with("Home"));
    assert!(rows[4].label.starts_with("Music"));
    assert!(rows[5].label.starts_with("Pictures"));
}

#[test]
fn network_activation_rejects_tokens_from_old_order_or_provider_availability() {
    let test = ui::TestUi::new();
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
    let calls = Rc::new(RefCell::new(Vec::new()));
    let captured = calls.clone();
    install(
        &test.window,
        presenter.clone(),
        Rc::new(move |command| {
            captured.borrow_mut().push(command);
            true
        }),
    );
    let context = |network_position| Event::DesktopContext {
        system_locations: Vec::new(),
        shell: ShellFlavor::Posix,
        network_position,
        locations: vec![NavigationLocation {
            id: "home".into(),
            label: "Home".into(),
            path: root.clone(),
            kind: LocationKind::Home,
            drive_letter: None,
        }],
    };
    presenter.borrow_mut().apply(context(1));
    presenter
        .borrow_mut()
        .apply(Event::NetworkAvailability(true));
    refresh(&test.window, &presenter.borrow());
    let token = || {
        test.window
            .get_navigation_choices()
            .iter()
            .find(|row| row.kind == midnight_ui::LocationRole::Network)
            .unwrap()
            .identity
    };
    let original = token();
    presenter.borrow_mut().apply(context(0));
    refresh(&test.window, &presenter.borrow());
    let reordered = token();
    assert_ne!(reordered, original);
    calls.borrow_mut().clear();
    test.window.invoke_navigation_choice(0, original);
    presenter
        .borrow_mut()
        .apply(Event::NetworkAvailability(false));
    refresh(&test.window, &presenter.borrow());
    test.window.invoke_navigation_choice(0, token());
    presenter
        .borrow_mut()
        .apply(Event::NetworkAvailability(true));
    refresh(&test.window, &presenter.borrow());
    test.window.invoke_navigation_choice(1, reordered);
    test.window
        .invoke_navigation_choice(1, "network:18446744073709551615".into());
    assert!(
        !calls
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::NetworkNavigate { .. }))
    );
    test.window.invoke_navigation_choice(1, token());
    assert!(calls.borrow().iter().any(|command| matches!(
        command,
        Command::NetworkNavigate {
            pane: PaneId::Right,
            action: NetworkAction::OpenRoot,
            ..
        }
    )));
}

#[test]
fn network_drivebar_opens_its_own_panel_and_disables_without_the_plugin() {
    let mut test = ui::TestUi::new();
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root)));
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::default())));
    let calls = Rc::new(RefCell::new(Vec::new()));
    let captured = calls.clone();
    install(
        &test.window,
        presenter.clone(),
        Rc::new(move |command| {
            captured.borrow_mut().push(command);
            true
        }),
    );
    presenter
        .borrow_mut()
        .apply(Event::NetworkAvailability(true));
    refresh(&test.window, &presenter.borrow());
    for swapped in [false, true] {
        test.window.set_panels_swapped(swapped);
        for active in [0, 1] {
            test.window.invoke_activate(1 - active);
            test.draw();
            calls.borrow_mut().clear();
            let label = if active == 0 {
                "Left panel Network Neighborhood"
            } else {
                "Right panel Network Neighborhood"
            };
            ui::click_root(&test.window, label, AccessibleRole::Button);
            assert!(calls.borrow().iter().any(|command| matches!(command,
                Command::NetworkNavigate { pane, action: NetworkAction::OpenRoot, .. }
                    if pane.index() == active as usize
            )));
        }
    }
    presenter
        .borrow_mut()
        .apply(Event::NetworkAvailability(false));
    refresh(&test.window, &presenter.borrow());
    test.draw();
    let button = ui::root_item(
        &test.window,
        "Left panel Network Neighborhood",
        AccessibleRole::Button,
    );
    assert_eq!(
        button
            .accessible_string_property(AccessibleStringProperty::Enabled)
            .as_deref(),
        Some("false")
    );
    calls.borrow_mut().clear();
    ui::click_root(
        &test.window,
        "Left panel Network Neighborhood",
        AccessibleRole::Button,
    );
    assert!(
        !calls
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::NetworkNavigate { .. }))
    );
    test.window.invoke_open_volume_menu(0);
    let row = ui::popup_item(
        &test.window,
        NETWORK_NEIGHBORHOOD_LABEL,
        AccessibleRole::Button,
    );
    assert_eq!(
        row.accessible_string_property(AccessibleStringProperty::Enabled)
            .as_deref(),
        Some("false")
    );
    ui::click_popup(
        &test.window,
        NETWORK_NEIGHBORHOOD_LABEL,
        AccessibleRole::Button,
    );
    assert!(
        !calls
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::NetworkNavigate { .. }))
    );
}

#[test]
fn network_menu_enter_double_click_and_refresh_use_captured_virtual_addresses() {
    let mut test = ui::TestUi::new();
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root)));
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::default())));
    let calls = Rc::new(RefCell::new(Vec::new()));
    let captured = calls.clone();
    let send: Rc<dyn Fn(Command) -> bool> = Rc::new(move |command| {
        captured.borrow_mut().push(command);
        true
    });
    install(&test.window, presenter.clone(), send.clone());
    refresh(&test.window, &presenter.borrow());
    let network = test
        .window
        .get_navigation_choices()
        .iter()
        .find(|row| row.kind == midnight_ui::LocationRole::Network)
        .unwrap();
    calls.borrow_mut().clear();
    test.window.invoke_navigation_choice(0, network.identity);
    assert!(
        !calls
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::NetworkNavigate { .. }))
    );
    presenter
        .borrow_mut()
        .apply(Event::NetworkAvailability(true));
    refresh(&test.window, &presenter.borrow());
    test.draw();
    test.window.invoke_open_volume_menu(0);
    ui::click_popup(
        &test.window,
        NETWORK_NEIGHBORHOOD_LABEL,
        AccessibleRole::Button,
    );
    assert!(calls.borrow().iter().any(|command| matches!(
        command,
        Command::NetworkNavigate {
            pane: PaneId::Left,
            action: NetworkAction::OpenRoot,
            ..
        }
    )));

    let publish = |window: &midnight_ui::MainWindow| {
        let navigation = calls
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::NetworkNavigate { navigation, .. } => Some(*navigation),
                _ => None,
            })
            .unwrap();
        let entries: Arc<[Entry]> = vec![Entry::from_network(
            EntryId(1),
            NetworkNode {
                address: address(2),
                display: "Server 文档".into(),
            },
        )]
        .into();
        presenter.borrow_mut().apply_with_sender(
            Event::NetworkUpdated {
                pane: PaneId::Left,
                navigation,
                update: Box::new(NetworkUpdate::Listing {
                    listing: NetworkListing {
                        address: address(1),
                        parent: None,
                        index: Arc::new(DirectoryIndex::from_entries(&entries)),
                        entries,
                    },
                    done: true,
                    error: None,
                }),
            },
            send.as_ref(),
        );
        refresh(window, &presenter.borrow());
    };
    publish(&test.window);
    test.draw();
    assert!(!test.window.get_left_native_filesystem());
    let x = test.window.get_left_list_left() + 100.0;
    let y = test.window.get_left_list_top() + test.window.get_left_view_row_height() * 1.5;
    calls.borrow_mut().clear();
    ui::click(&test.window, x, y);
    ui::key(&test.window, Key::Return);
    assert!(calls.borrow().iter().any(|command| matches!(
        command,
        Command::NetworkNavigate { action: NetworkAction::Activate(target), .. }
            if target == &address(2)
    )));

    publish(&test.window);
    test.draw();
    calls.borrow_mut().clear();
    // The preceding single click belonged to the old view. Start a new native
    // double-click sequence, then keep the two presses within its interval.
    std::thread::sleep(std::time::Duration::from_millis(650));
    ui::click(&test.window, x, y);
    test.draw();
    std::thread::sleep(std::time::Duration::from_millis(70));
    ui::click(&test.window, x, y);
    assert!(calls.borrow().iter().any(|command| matches!(
        command,
        Command::NetworkNavigate { action: NetworkAction::Activate(target), .. }
            if target == &address(2)
    )));
    publish(&test.window);
    test.draw();
    calls.borrow_mut().clear();
    test.window.invoke_action(0, 3);
    assert!(calls.borrow().iter().any(|command| matches!(
        command,
        Command::NetworkNavigate { action: NetworkAction::Refresh(target), .. }
            if target == &address(1)
    )));
    assert!(!calls.borrow().iter().any(|command| matches!(
        command,
        Command::Navigate { .. } | Command::Refresh { .. } | Command::OpenEntry { .. }
    )));
}
