//! Exercise Network through the installed menu, pointer and keyboard bridge.
use crate::support::ui;

use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::ItemRc,
    items::{AccessibleRole, ComplexText, Orientation, SimpleText},
    window::WindowInner,
};
use midnight_core::*;
use midnight_ui::{Presenter, install, refresh};
use slint::{
    ComponentHandle, Model,
    platform::{Key, WindowAdapter},
};
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
    // Slint's 500 ms double-click interval follows this clock, not scheduling delays.
    let (mut test, clock) = ui::TestUi::with_virtual_clock();
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
                        ancestors: Arc::default(),
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
    // A fresh single click: beyond the double-click interval of any earlier press.
    ui::advance(&clock, std::time::Duration::from_millis(650));
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
    ui::advance(&clock, std::time::Duration::from_millis(650));
    ui::click(&test.window, x, y);
    test.draw();
    ui::advance(&clock, std::time::Duration::from_millis(70));
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

/// A server below a workgroup, a provider and the root (0), with captions long enough
/// that a narrow panel shortens the path.
fn chain_address(level: u8) -> NetworkAddress {
    let captions = [
        "Network",
        "Microsoft Windows Network",
        "A rather long workgroup name 文档",
        "SERVER-0042",
    ];
    NetworkAddress {
        provider: "network.test".into(),
        opaque: Arc::from([7, level]),
        display: captions[usize::from(level)].into(),
    }
}

fn visit(item: ItemRc, found: &mut dyn FnMut(&ItemRc)) {
    found(&item);
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        visit(item, found);
    }
}

/// Visible buttons above the left list whose accessible label passes `keep`, left to
/// right.
fn left_bar_buttons(test: &ui::TestUi, keep: &dyn Fn(&str) -> bool) -> Vec<(String, ItemRc)> {
    let mut found = Vec::new();
    visit(
        ItemRc::new_root(WindowInner::from_pub(test.window.window()).component()),
        &mut |item| {
            if item.is_visible()
                && item.accessible_role() == AccessibleRole::Button
                && let Some(label) =
                    item.accessible_string_property(AccessibleStringProperty::Label)
                && keep(&label)
            {
                found.push((label.to_string(), item.clone()));
            }
        },
    );
    let left = test.window.get_left_list_left();
    let right = left + test.window.get_left_list_width();
    let top = test.window.get_left_list_top();
    let origin = |item: &ItemRc| item.map_to_native_window(item.geometry().origin);
    found.retain(|(_, item)| {
        let at = origin(item);
        at.x >= left - 0.5 && at.x < right && at.y < top
    });
    found.sort_by(|(_, a), (_, b)| origin(a).x.total_cmp(&origin(b).x));
    found
}

/// The shown parts of the left path bar, which shows `shown`, by the prefix each opens
/// (its accessible label is "Open <prefix>").
fn shown_parts(test: &ui::TestUi, shown: &str) -> Vec<(String, ItemRc)> {
    left_bar_buttons(test, &|label| {
        label
            .strip_prefix("Open ")
            .is_some_and(|prefix| shown.starts_with(prefix))
    })
    .into_iter()
    .map(|(label, item)| (label["Open ".len()..].to_owned(), item))
    .collect()
}

/// The shown markers of omitted parts in the left path bar.
fn markers(test: &ui::TestUi) -> usize {
    left_bar_buttons(test, &|label| label == "Edit full folder path").len()
}

/// Whether a part shows its whole caption.
fn whole(test: &ui::TestUi, part: &ItemRc) -> bool {
    let mut caption = None;
    visit(part.clone(), &mut |item| {
        if caption.is_none()
            && (item.downcast::<SimpleText>().is_some() || item.downcast::<ComplexText>().is_some())
        {
            caption = Some(item.clone());
        }
    });
    let caption = caption.expect("a part has a caption");
    let adapter: Rc<dyn WindowAdapter> = test.native.clone();
    let rect = caption.geometry();
    let natural = caption
        .borrow()
        .as_ref()
        .layout_info(Orientation::Horizontal, rect.height(), &adapter, &caption)
        .preferred;
    rect.width() + 0.5 >= natural
}

#[test]
fn the_network_path_bar_shows_and_browses_each_published_ancestor() {
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
    let network = test
        .window
        .get_navigation_choices()
        .iter()
        .find(|row| row.kind == midnight_ui::LocationRole::Network)
        .unwrap();
    test.window.invoke_navigation_choice(0, network.identity);
    let navigation = calls
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::NetworkNavigate { navigation, .. } => Some(*navigation),
            _ => None,
        })
        .expect("the menu opens Network");
    // The provider publishes the server with its ancestors.
    presenter.borrow_mut().apply(Event::NetworkUpdated {
        pane: PaneId::Left,
        navigation,
        update: Box::new(NetworkUpdate::Listing {
            listing: NetworkListing {
                address: chain_address(3),
                ancestors: (0..3).map(chain_address).collect(),
                entries: Arc::default(),
                index: Arc::default(),
            },
            done: true,
            error: None,
        }),
    });
    let prefixes: Vec<String> = (0..4)
        .map(|level| {
            (0..=level)
                .map(|level| chain_address(level).display)
                .collect::<Vec<_>>()
                .join(" › ")
        })
        .collect();
    let shown = prefixes[3].clone();
    // A narrow panel keeps the root and the server whole and omits the middle.
    test.window.set_panel_ratio(0.0);
    refresh(&test.window, &presenter.borrow());
    test.draw();
    let parts = shown_parts(&test, &shown);
    let names: Vec<&str> = parts.iter().map(|(prefix, _)| prefix.as_str()).collect();
    assert_eq!(names, [prefixes[0].as_str(), shown.as_str()], "narrow");
    for (prefix, part) in &parts {
        assert!(whole(&test, part), "{prefix} is cut");
    }
    assert_eq!(markers(&test), 1, "the omitted middle has a marker");
    // A wide panel shows every part; each browses its location.
    test.set_size(2400, 820);
    test.window.set_panel_ratio(1.0);
    refresh(&test.window, &presenter.borrow());
    test.draw();
    let names: Vec<String> = shown_parts(&test, &shown)
        .into_iter()
        .map(|(prefix, _)| prefix)
        .collect();
    assert_eq!(names, prefixes, "wide");
    assert_eq!(markers(&test), 0);
    calls.borrow_mut().clear();
    ui::click_root(
        &test.window,
        &format!("Open {}", prefixes[1]),
        AccessibleRole::Button,
    );
    assert!(
        matches!(&calls.borrow()[..], [Command::NetworkNavigate {
            pane: PaneId::Left, action: NetworkAction::Browse(target), ..
        }] if *target == chain_address(1)),
        "{:?}",
        calls.borrow()
    );
}
