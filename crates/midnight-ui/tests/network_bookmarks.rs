//! Real shortcut routing reopens saved Network addresses without native-path parsing.
#[path = "support/ui.rs"]
mod ui;

use midnight_core::*;
use midnight_ui::{Presenter, install, refresh};
use slint::platform::Key;
use std::{cell::RefCell, rc::Rc, sync::Arc};

#[test]
fn restored_network_hot_path_remapping_routes_exact_address_to_either_pane() {
    let mut test = ui::TestUi::new();
    let target = NetworkAddress {
        provider: "network.missing-plugin".into(),
        opaque: Arc::from([0, 128, 255, 1]),
        display: "Caption @ not a UNC path".into(),
    };
    let preferences = UserPreferences {
        bookmarks: vec![Bookmark {
            id: BookmarkId(91),
            name: "Network".into(),
            path: std::env::temp_dir(),
            archive: None,
            network: Some(target.clone()),
            slot: Some(1),
        }],
        shortcuts: vec![ShortcutBinding {
            action: "bookmark.open.1".into(),
            keys: vec!["Ctrl+Shift+F12".into()],
        }],
        ..Default::default()
    };
    preferences.validate().unwrap();
    let presenter = Rc::new(RefCell::new(Presenter::new(
        std::env::temp_dir(),
        std::env::temp_dir(),
    )));
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::new(preferences))));
    presenter
        .borrow_mut()
        .apply(Event::NetworkAvailability(false));
    let sent = Rc::new(RefCell::new(Vec::new()));
    let out = sent.clone();
    install(
        &test.window,
        presenter.clone(),
        Rc::new(move |command| {
            out.borrow_mut().push(command);
            true
        }),
    );
    refresh(&test.window, &presenter.borrow());
    for pane in [PaneId::Left, PaneId::Right] {
        test.window.invoke_activate(pane.index() as i32);
        test.window.invoke_focus_files();
        sent.borrow_mut().clear();
        assert!(
            !test
                .window
                .invoke_physical_shortcut("Digit1".into(), false, false, false, false)
        );
        assert!(
            !sent
                .borrow()
                .iter()
                .any(|command| matches!(command, Command::NetworkNavigate { .. }))
        );
        ui::chord(&test.window, &[Key::Control, Key::Shift], Key::F12);
        test.draw();
        let commands = sent.borrow();
        let requests: Vec<_> = commands
            .iter()
            .filter_map(|command| match command {
                Command::NetworkNavigate {
                    pane,
                    action: NetworkAction::Browse(address),
                    ..
                } => Some((*pane, address)),
                _ => None,
            })
            .collect();
        assert_eq!(requests, [(pane, &target)]);
        assert!(!commands.iter().any(|command| matches!(
            command,
            Command::Navigate { .. }
                | Command::NetworkNavigate {
                    action: NetworkAction::Activate(_) | NetworkAction::Parse(_),
                    ..
                }
        )));
    }
}
