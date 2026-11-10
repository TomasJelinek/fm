//! Loading feedback remains visible before the backend sends its first page.
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{item_tree::ItemRc, window::WindowInner};
use midnight_core::*;
use midnight_ui::{Presenter, install, refresh};
use slint::{ComponentHandle, Model};
use std::{cell::RefCell, rc::Rc, sync::Arc};

fn visible_text(window: &midnight_ui::MainWindow) -> Vec<slint::SharedString> {
    let inner = WindowInner::from_pub(window.window());
    ui::visible_text_values(ItemRc::new_root(inner.component()))
}

#[test]
fn installed_navigation_shows_loading_with_parent_rows_refresh_rows_and_pending_network() {
    let mut test = ui::TestUi::new();
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
