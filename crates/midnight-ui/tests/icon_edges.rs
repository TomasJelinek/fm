//! Native icons are looked up at the edge of the place that draws them.
use crate::support::ui;

use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::ItemRc,
    items::{AccessibleRole, ImageItem},
    window::WindowInner,
};
use midnight_core::*;
use midnight_ui::{IconView, Presenter, icon_view, install, refresh};
use slint::Model;
use std::{cell::RefCell, rc::Rc, sync::Arc};

fn image(item: ItemRc) -> Option<ItemRc> {
    if item.is_visible() && item.downcast::<ImageItem>().is_some() {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = image(item) {
            return Some(found);
        }
    }
    None
}

/// Every visible parent row, left pane first.
fn parent_rows(item: ItemRc, found: &mut Vec<ItemRc>) {
    if item.is_visible()
        && item.accessible_role() == AccessibleRole::ListItem
        && item
            .accessible_string_property(AccessibleStringProperty::Label)
            .is_some_and(|label| label.as_str() == "..")
    {
        found.push(item);
        return;
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        parent_rows(item, found);
    }
}

fn icon(edge: u16) -> Arc<IconPixels> {
    let edge = u32::from(edge);
    Arc::new(IconPixels::new(edge, edge, vec![255; (edge * edge * 4) as usize]).unwrap())
}

#[test]
fn rows_and_tiles_each_draw_the_bitmap_of_their_own_size_one_to_one() {
    let mut test = ui::TestUi::new();
    let path = std::env::current_dir().unwrap();
    let presenter = Rc::new(RefCell::new(Presenter::new(path.clone(), path)));
    // The left pane lists rows, the right pane tiles. Previews stay off, so
    // every tile shows its native icon.
    let mut preferences = UserPreferences::default();
    preferences.panel_view_modes[1] = PanelViewMode::Thumbnails;
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::new(
            AcceptedPreferences {
                preferences,
                comparators: Default::default(),
            },
        ))));
    let navigations = Rc::new(RefCell::new([None; 2]));
    let edges = Rc::new(RefCell::new(Vec::new()));
    let (captured, sent) = (navigations.clone(), edges.clone());
    let send: Rc<dyn Fn(Command) -> bool> = Rc::new(move |command| {
        match command {
            Command::Navigate {
                pane, navigation, ..
            } => captured.borrow_mut()[pane.index()] = Some(navigation),
            Command::SetIconEdges(edges) => sent.borrow_mut().push(edges),
            _ => {}
        }
        true
    });
    install(&test.window, presenter.clone(), send.clone());
    for pane in [PaneId::Left, PaneId::Right] {
        presenter.borrow_mut().apply(Event::ListingFinished {
            pane,
            navigation: navigations.borrow()[pane.index()].unwrap(),
            entries: Arc::from([]),
            index: Arc::new(DirectoryIndex::from_entries(&[])),
        });
    }
    refresh(&test.window, &presenter.borrow());
    test.draw();
    let view = icon_view(&test.window);
    assert_eq!(
        view,
        IconView {
            edges: IconEdges {
                entry: 20,
                tile: 96,
                location: 20,
                action: 32,
            },
            tiles: [false, true],
        }
    );
    assert!(presenter.borrow_mut().sync_icon_view(view, &*send));
    assert_eq!(*edges.borrow(), [view.edges]);

    // The backend answers with the parent row's art at every edge it is drawn at.
    let parent = IconKey::action(ActionIcon::Parent);
    for edge in view.edges.of(parent.kind) {
        presenter.borrow_mut().apply(Event::IconLoaded {
            key: parent.clone().sized(edge),
            pixels: icon(edge),
        });
    }
    refresh(&test.window, &presenter.borrow());
    test.draw();
    let rows = [
        test.window.get_left_rows().row_data(0).unwrap(),
        test.window.get_right_rows().row_data(0).unwrap(),
    ];
    assert!(rows.iter().all(|row| row.parent_entry));
    assert_eq!(rows[0].icon.size().width, 20);
    assert_eq!(rows[1].icon.size().width, 96);

    // Each bitmap fills a box of exactly its own size: nothing is resampled.
    let mut drawn = Vec::new();
    let root = WindowInner::from_pub(slint::ComponentHandle::window(&test.window)).component();
    parent_rows(ItemRc::new_root(root), &mut drawn);
    assert_eq!(drawn.len(), 2, "one parent row per pane");
    drawn.sort_by(|left, right| {
        let x = |item: &ItemRc| item.map_to_native_window(item.geometry().origin).x;
        x(left).total_cmp(&x(right))
    });
    for (row, edge) in drawn.into_iter().zip([20.0, 96.0]) {
        let artwork = image(row).expect("parent artwork");
        let source = artwork.downcast::<ImageItem>().unwrap();
        assert_eq!(source.as_pin_ref().source().size().width as f32, edge);
        let size = artwork.geometry().size;
        assert_eq!((size.width, size.height), (edge, edge));
    }

    // Toolbar actions keep the bitmap of the default edge.
    assert_eq!(test.window.get_action_icons().parent.size().width, 32);
}
