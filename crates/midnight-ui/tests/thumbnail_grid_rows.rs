//! The Thumbnails grid draws the row its viewport cuts at an edge and requests
//! that row's thumbnails, and those of one row beyond the edge.
use crate::support::ui;
use i_slint_core::{
    accessibility::AccessibleStringProperty, item_tree::ItemRc, items::AccessibleRole,
    window::WindowInner,
};
use midnight_core::*;
use midnight_ui::{MainWindow, Presenter, install, refresh};
use slint::{ComponentHandle, Model};
use std::{cell::RefCell, rc::Rc, sync::Arc};

/// The grid cell that shows `name`, if the panel instantiated it.
fn tile(window: &MainWindow, name: &str) -> Option<ItemRc> {
    fn find(item: ItemRc, name: &str) -> Option<ItemRc> {
        if item.accessible_role() == AccessibleRole::ListItem
            && item
                .accessible_string_property(AccessibleStringProperty::Label)
                .is_some_and(|label| label.as_str() == name)
        {
            return Some(item);
        }
        let mut child = item.first_child();
        while let Some(item) = child {
            child = item.next_sibling();
            if let Some(found) = find(item, name) {
                return Some(found);
            }
        }
        None
    }
    find(
        ItemRc::new_root(WindowInner::from_pub(window.window()).component()),
        name,
    )
}

#[test]
fn a_row_the_viewport_cuts_is_drawn_and_its_thumbnails_are_requested() {
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
        .apply(Event::PreferencesLoaded(Ok(Box::new(
            AcceptedPreferences {
                preferences: UserPreferences {
                    panel_view_modes: [PanelViewMode::Thumbnails; 2],
                    thumbnails_network: true,
                    ..Default::default()
                },
                comparators: FileComparatorSnapshot::default(),
            },
        ))));
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
    let entries: Arc<[Entry]> = (0..60)
        .map(|id| {
            Entry::new(
                EntryId(id),
                root.join(format!("tile-{id:02}.png")),
                EntryKind::File,
            )
        })
        .collect();
    presenter.borrow_mut().apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation,
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
    });
    refresh(&ui.window, &presenter.borrow());
    ui.draw();

    // Size the window so that the panel's last visible row is cut in half.
    let row = ui.window.get_left_view_row_height();
    let visible = ui.window.get_left_list_visible_height();
    let whole = (visible / row).floor();
    assert!(whole >= 1.0, "a {visible} px panel shows a {row} px row");
    let height = ui.height as f32 + whole * row + row / 2.0 - visible;
    ui.resize(ui.width, height.round() as usize);
    refresh(&ui.window, &presenter.borrow());
    ui.draw();
    let visible = ui.window.get_left_list_visible_height();
    let cut = (visible / row).floor();
    let shown = visible - cut * row;
    assert!(
        shown > row * 0.3 && shown < row * 0.7,
        "{shown} px of a {row} px row are visible"
    );
    let cut = cut as usize;
    let columns = usize::try_from(ui.window.get_left_view_columns()).unwrap();
    let rows = ui.window.get_left_rows();
    // The parent row is the grid's first cell.
    let names = |band: usize| -> Vec<String> {
        (band * columns..(band + 1) * columns)
            .filter_map(|index| rows.row_data(index))
            .filter(|row| !row.parent_entry)
            .map(|row| row.name.to_string())
            .collect()
    };
    let cut_names = names(cut);
    assert_eq!(cut_names.len(), columns, "the cut row is a full grid row");
    for name in &cut_names {
        assert!(
            tile(&ui.window, name).is_some_and(|item| item.is_visible()),
            "{name}, in the row the viewport cuts, is drawn"
        );
    }

    // Requests for the reported rows, as the application's thumbnail pass sends.
    let range = (
        usize::try_from(ui.window.get_left_thumbnail_row_start()).unwrap(),
        usize::try_from(ui.window.get_left_thumbnail_row_end()).unwrap(),
    );
    let targets = presenter
        .borrow_mut()
        .thumbnail_targets([range, (0, 0)], ["", ""]);
    let requested = |name: &str| {
        let id: u64 = name["tile-".len()..name.len() - ".png".len()]
            .parse()
            .unwrap();
        let entry = format!(":entry:{id}:");
        targets
            .iter()
            .any(|target| target.identity.contains(&entry))
    };
    for name in &cut_names {
        assert!(requested(name), "{name}, in the cut row, is requested");
    }
    let ahead = names(cut + 1);
    assert!(!ahead.is_empty());
    for name in &ahead {
        assert!(
            requested(name),
            "{name}, one row below the edge, is requested"
        );
    }
    assert!(
        !names(cut + 2).iter().any(|name| requested(name)),
        "the look-ahead is one row"
    );
}
