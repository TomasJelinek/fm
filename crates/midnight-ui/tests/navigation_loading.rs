//! Loading feedback is delayed without delaying the backend request or changing row geometry.
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{item_tree::ItemRc, window::WindowInner};
use midnight_core::*;
use midnight_ui::{Presenter, install, refresh};
use slint::{ComponentHandle, Model};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
    time::Duration,
};

fn visible_text(window: &midnight_ui::MainWindow) -> Vec<slint::SharedString> {
    let inner = WindowInner::from_pub(window.window());
    ui::visible_text_values(ItemRc::new_root(inner.component()))
}

#[test]
fn installed_navigation_shows_loading_with_parent_rows_refresh_rows_and_pending_network() {
    let clock = Rc::new(Cell::new(Duration::ZERO));
    let mut test = ui::TestUi::with_clock(Some(clock.clone()));
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target = root.join("delayed folder");
    let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
    presenter
        .borrow_mut()
        .apply(Event::VolumesUpdated(vec![Volume {
            id: "native-volume".into(),
            label: "Native volume".into(),
            root,
        }]));
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
    test.window
        .invoke_navigate(0, target.to_string_lossy().into_owned().into());
    let navigation = calls
        .borrow()
        .iter()
        .rev()
        .find_map(|command| match command {
            Command::Navigate {
                pane: PaneId::Left,
                navigation,
                ..
            } => Some(*navigation),
            _ => None,
        })
        .unwrap();
    let native_loading = format!("Loading {}…", target.to_string_lossy());
    test.draw();
    assert!(test.window.get_left_loading());
    assert!(
        test.window.get_left_rows().row_count() > 0,
        "the parent row is already present"
    );
    assert!(
        !visible_text(&test.window)
            .iter()
            .any(|text| text == &native_loading)
    );
    clock.set(clock.get() + Duration::from_millis(499));
    test.draw();
    assert!(
        !visible_text(&test.window)
            .iter()
            .any(|text| text == &native_loading)
    );
    clock.set(clock.get() + Duration::from_millis(1));
    test.draw();
    assert!(
        visible_text(&test.window)
            .iter()
            .any(|text| text == &native_loading)
    );
    render::snapshot(&test, "navigation-loading-native");

    let entries: Arc<[Entry]> = vec![Entry::new(
        EntryId(1),
        target.join("existing"),
        EntryKind::Directory,
    )]
    .into();
    presenter.borrow_mut().apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation,
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
    });
    refresh(&test.window, &presenter.borrow());
    test.draw();
    assert!(!test.window.get_left_loading());
    assert!(
        !visible_text(&test.window)
            .iter()
            .any(|text| text == &native_loading)
    );

    let list_top = test.window.get_left_list_top();
    let list_height = test.window.get_left_list_height();
    test.window.invoke_action(0, 3);
    test.draw();
    assert_eq!(test.window.get_left_list_top(), list_top);
    assert_eq!(test.window.get_left_list_height(), list_height);
    assert!(
        !visible_text(&test.window)
            .iter()
            .any(|text| text == &native_loading)
    );
    clock.set(clock.get() + Duration::from_millis(500));
    test.draw();
    render::snapshot(&test, "navigation-loading-refresh");
    assert!(
        test.window.get_left_rows().row_count() > 1,
        "refresh retains committed rows"
    );
    assert!(
        visible_text(&test.window)
            .iter()
            .any(|text| text == &native_loading)
    );

    let refresh_navigation = calls
        .borrow()
        .iter()
        .rev()
        .find_map(|command| match command {
            Command::Refresh { navigation, .. } => Some(*navigation),
            _ => None,
        })
        .unwrap();
    presenter.borrow_mut().apply(Event::ListingFailed {
        pane: PaneId::Left,
        navigation: refresh_navigation,
        error: FsError::from_io("list", &target, std::io::ErrorKind::PermissionDenied.into()),
    });
    refresh(&test.window, &presenter.borrow());
    test.draw();
    assert!(!test.window.get_left_loading());
    assert!(!test.window.get_left_error().is_empty());
    assert!(test.window.get_left_rows().row_count() > 1);
    assert!(
        !visible_text(&test.window)
            .iter()
            .any(|text| text == &native_loading)
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
    test.draw();
    assert_eq!(test.window.get_left_path(), NETWORK_NEIGHBORHOOD_LABEL);
    assert_eq!(test.window.get_left_rows().row_count(), 0);
    assert!(!test.window.get_left_native_filesystem());
    assert_eq!(
        test.window
            .get_navigation_choices()
            .row_data(test.window.get_left_navigation_index() as usize)
            .unwrap()
            .kind,
        midnight_ui::LocationRole::Network
    );
    let selected: Vec<_> = test
        .window
        .get_left_locations()
        .iter()
        .filter(|row| row.selected)
        .collect();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].kind, midnight_ui::LocationRole::Network);
    assert!(
        !visible_text(&test.window)
            .iter()
            .any(|text| text == &format!("Loading {NETWORK_NEIGHBORHOOD_LABEL}…"))
    );
    clock.set(clock.get() + Duration::from_millis(500));
    test.draw();
    assert!(
        visible_text(&test.window)
            .iter()
            .any(|text| text == &format!("Loading {NETWORK_NEIGHBORHOOD_LABEL}…"))
    );
    render::snapshot(&test, "navigation-loading-network");
    test.window.set_command_text("keep draft".into());
    test.window.invoke_insert_path(0);
    test.window.invoke_insert_name();
    assert_eq!(test.window.get_command_text(), "keep draft");
}

#[test]
fn denied_folder_open_restores_cursor_and_exact_viewport_after_layout_settles() {
    let mut test = ui::TestUi::new();
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
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
    // This viewport harness has one adapter. Separate notice-window tests render the modal.
    let notice_opens = Rc::new(Cell::new(0));
    let observed = notice_opens.clone();
    test.window
        .on_open_notice_window(move || observed.set(observed.get() + 1));
    let latest_navigation = || {
        calls
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::Navigate {
                    pane: PaneId::Left,
                    navigation,
                    ..
                } => Some(*navigation),
                _ => None,
            })
            .unwrap()
    };
    let entries: Arc<[Entry]> = (0..200)
        .map(|index| {
            Entry::new(
                EntryId(index + 1),
                root.join(format!("folder-{index:03}")),
                EntryKind::Directory,
            )
        })
        .collect::<Vec<_>>()
        .into();
    presenter.borrow_mut().apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation: latest_navigation(),
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
    });
    refresh(&test.window, &presenter.borrow());
    test.draw();
    test.window.invoke_row_click(0, 81, false, false);
    test.window.invoke_scroll_selection(0, 100000.0);
    test.window
        .invoke_scroll_selection(0, -77.0 * test.window.get_left_view_row_height());
    for _ in 0..3 {
        test.draw();
    }
    let offset = test.window.get_left_list_content_y();
    assert!(offset < -1000.0);
    assert_eq!(test.window.get_left_cursor(), 81);
    for route in 0..2 {
        let previous = latest_navigation();
        test.window.invoke_focus_files();
        if route == 0 {
            let point = ui::root_position(
                &test.window,
                "folder-080",
                i_slint_core::items::AccessibleRole::ListItem,
            );
            ui::click(&test.window, point.x, point.y);
            ui::click(&test.window, point.x, point.y);
        } else {
            ui::key(&test.window, slint::platform::Key::Return);
        }
        let failed = latest_navigation();
        assert_ne!(
            failed, previous,
            "route {route} must issue a new navigation"
        );
        test.draw();
        assert!(test.window.get_left_loading());
        presenter.borrow_mut().apply(Event::ListingFailed {
            pane: PaneId::Left,
            navigation: failed,
            error: FsError::from_io(
                "list",
                &root.join("folder-080"),
                std::io::Error::new(std::io::ErrorKind::PermissionDenied, "Access denied"),
            ),
        });
        refresh(&test.window, &presenter.borrow());
        for frame in 0..5 {
            test.draw();
            assert_eq!(test.window.get_left_cursor(), 81);
            assert!(
                (test.window.get_left_list_content_y() - offset).abs() < 0.5,
                "denied navigation moved the viewport on frame {frame}: before={offset}, after={}",
                test.window.get_left_list_content_y(),
            );
        }
        assert!(test.window.get_left_error().is_empty());
        assert_eq!(
            notice_opens.get(),
            route + 1,
            "each explicit failed open requests a message box"
        );
        assert_eq!(
            test.window.get_message_body(),
            "Access to this folder was denied."
        );
        assert_eq!(
            test.window.get_message_path().as_str(),
            root.join("folder-080").to_string_lossy()
        );
        assert_eq!(test.window.get_left_path().as_str(), root.to_string_lossy());
    }
    render::snapshot(&test, "navigation-denied-preserved-viewport");
    test.window
        .invoke_scroll_selection(0, -test.window.get_left_view_row_height());
    test.draw();
    let scrolled = test.window.get_left_list_content_y();
    assert!(scrolled < offset);
    refresh(&test.window, &presenter.borrow());
    test.draw();
    assert_eq!(
        test.window.get_left_list_content_y(),
        scrolled,
        "a later refresh must not undo the user's scroll"
    );
    let stale = latest_navigation();
    test.window.invoke_open(0, 81);
    presenter.borrow_mut().apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation: latest_navigation(),
        index: Arc::new(DirectoryIndex::from_entries(&[])),
        entries: Arc::from([]),
    });
    refresh(&test.window, &presenter.borrow());
    for _ in 0..3 {
        test.draw();
    }
    assert_eq!(test.window.get_left_list_content_y(), 0.0);
    assert!(test.window.get_left_error().is_empty());
    presenter.borrow_mut().apply(Event::ListingFailed {
        pane: PaneId::Left,
        navigation: stale,
        error: FsError::from_io("list", &root, std::io::Error::other("stale denial")),
    });
    refresh(&test.window, &presenter.borrow());
    test.draw();
    assert_eq!(test.window.get_left_list_content_y(), 0.0);
    assert!(test.window.get_left_error().is_empty());
    render::snapshot(&test, "navigation-denied-then-success");
}
