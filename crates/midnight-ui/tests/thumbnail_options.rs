//! Thumbnail size is a reversible Options preview and reflows both native panel grids.
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{
    item_tree::ItemRc,
    items::{AccessibleRole, ImageItem},
};
use midnight_core::*;
use midnight_ui::{Presenter, install, refresh};
use slint::{Model, platform::Key};
use std::{cell::RefCell, rc::Rc, sync::Arc};

fn artwork_edge(item: ItemRc) -> f32 {
    let mut edge = if item.downcast::<ImageItem>().is_some() {
        item.geometry().width()
    } else {
        0.0
    };
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        edge = edge.max(artwork_edge(item));
    }
    edge
}

#[test]
fn actual_size_control_previews_cancel_restores_and_ok_persists() {
    let mut ui = ui::TestUi::new();
    let root = std::env::current_dir().unwrap();
    let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
    let sent = Rc::new(RefCell::new(Vec::new()));
    let captured = sent.clone();
    install(
        &ui.window,
        presenter.clone(),
        Rc::new(move |command| {
            captured.borrow_mut().push(command);
            true
        }),
    );
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::new(UserPreferences {
            panel_view_modes: [PanelViewMode::Thumbnails; 2],
            thumbnails_network: true,
            ..Default::default()
        }))));
    let navigation = sent
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
    let entries: Arc<[Entry]> = vec![Entry::new(
        EntryId(1),
        root.join("preview-fixture.png"),
        EntryKind::File,
    )]
    .into();
    presenter.borrow_mut().apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation,
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
    });
    let targets = presenter
        .borrow_mut()
        .thumbnail_targets([(0, 10), (0, 0)], ["", ""]);
    let identity = targets[0].identity.clone();
    assert!(presenter.borrow_mut().receive_thumbnail(
        &identity,
        0,
        96,
        48,
        &vec![180; 96 * 48 * 4]
    ));
    refresh(&ui.window, &presenter.borrow());
    ui.draw();
    let original_height = ui.window.get_left_view_row_height();
    let original_columns = ui.window.get_left_view_columns();
    assert_eq!(ui.window.get_thumbnail_size(), 72);
    assert_eq!(
        artwork_edge(ui::root_item(
            &ui.window,
            "preview-fixture.png",
            AccessibleRole::ListItem
        )),
        72.0
    );
    sent.borrow_mut().clear();
    for cancel in [true, false] {
        ui.window.set_options_topic_index(6);
        ui.window.invoke_show_options();
        ui.draw();
        ui::click_popup(&ui.window, "Thumbnail size", AccessibleRole::Combobox);
        for size in [96, 128, 192, 256] {
            ui::key(&ui.window, Key::DownArrow);
            assert_eq!(ui.window.get_thumbnail_size(), size);
        }
        ui::key(&ui.window, Key::Return);
        ui.draw();
        assert_eq!(ui.window.get_thumbnail_size(), 256);
        assert_eq!(
            ui.window.get_left_view_row_height(),
            original_height + 184.0
        );
        assert_eq!(
            ui.window.get_right_view_row_height(),
            original_height + 184.0
        );
        assert!(ui.window.get_left_view_columns() < original_columns);
        assert_eq!(
            artwork_edge(ui::root_item(
                &ui.window,
                "preview-fixture.png",
                AccessibleRole::ListItem
            )),
            256.0
        );
        let rows = ui.window.get_left_rows();
        let row = (0..rows.row_count())
            .filter_map(|index| rows.row_data(index))
            .find(|row| row.name == "preview-fixture.png")
            .unwrap();
        assert_eq!(
            row.icon.size().width,
            96,
            "the resident fallback remains until an upgrade arrives"
        );
        assert!(
            !sent
                .borrow()
                .iter()
                .any(|command| matches!(command, Command::SavePreferences { .. }))
        );
        render::snapshot(&ui, "thumbnail-size-preview");
        ui::click_popup(
            &ui.window,
            if cancel { "Cancel" } else { "OK" },
            AccessibleRole::Button,
        );
        ui.draw();
        if cancel {
            assert_eq!(ui.window.get_thumbnail_size(), 72);
            assert_eq!(ui.window.get_left_view_row_height(), original_height);
        }
    }
    let saved = sent
        .borrow()
        .iter()
        .filter_map(|command| match command {
            Command::SavePreferences {
                revision,
                preferences,
            } => Some((*revision, preferences.thumbnail_size)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].1, 256);
    presenter.borrow_mut().apply(Event::PreferencesSaved {
        revision: saved[0].0,
        result: Ok(()),
    });
    refresh(&ui.window, &presenter.borrow());
    ui.draw();
    assert!(!ui::has_popup(&ui.window));
    assert!(presenter.borrow_mut().receive_thumbnail_at_resolution(
        &identity,
        0,
        256,
        128,
        256,
        &vec![255; 256 * 128 * 4]
    ));
    refresh(&ui.window, &presenter.borrow());
    ui.draw();
    let rows = ui.window.get_left_rows();
    let row = (0..rows.row_count())
        .filter_map(|index| rows.row_data(index))
        .find(|row| row.name == "preview-fixture.png")
        .unwrap();
    assert_eq!(row.icon.size().width, 256);
    render::snapshot(&ui, "thumbnail-size-upgraded");
    ui.window.invoke_show_options();
    ui.draw();
    ui::click_popup(&ui.window, "Reset to defaults", AccessibleRole::Button);
    ui.draw();
    assert_eq!(ui.window.get_thumbnail_size(), 72);
    ui::click_popup(&ui.window, "Cancel", AccessibleRole::Button);
    ui.draw();
    assert_eq!(ui.window.get_thumbnail_size(), 256);
}
