//! Synthetic navigation rows keep semantic artwork across every panel layout.
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{
    item_tree::ItemRc,
    items::{AccessibleRole, ImageItem},
};
use midnight_core::*;
use midnight_ui::{Presenter, install, refresh};
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

#[test]
fn synthetic_parent_uses_an_arrow_before_and_after_native_artwork_arrives_in_every_mode() {
    let mut test = ui::TestUi::new();
    let path = std::env::current_dir().unwrap();
    let presenter = Rc::new(RefCell::new(Presenter::new(path.clone(), path)));
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::default())));
    let navigation = Rc::new(RefCell::new(None));
    let captured = navigation.clone();
    install(
        &test.window,
        presenter.clone(),
        Rc::new(move |command| {
            if let Command::Navigate {
                pane: PaneId::Left,
                navigation,
                ..
            } = command
            {
                *captured.borrow_mut() = Some(navigation);
            }
            true
        }),
    );
    presenter.borrow_mut().apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation: navigation.borrow().unwrap(),
        entries: Arc::from([]),
        index: Arc::new(DirectoryIndex::from_entries(&[])),
    });
    // A loaded generic folder must never be mistaken for navigation artwork.
    presenter.borrow_mut().apply(Event::IconLoaded {
        key: IconKey::folder(),
        pixels: Arc::new(IconPixels::new(2, 2, vec![255; 16]).unwrap()),
    });
    refresh(&test.window, &presenter.borrow());
    let row = test.window.get_left_rows().row_data(0).unwrap();
    assert!(row.parent_entry);
    assert_eq!(row.icon.size().width, 0);
    for native in [false, true] {
        if native {
            presenter.borrow_mut().apply(Event::IconLoaded {
                key: IconKey::action(ActionIcon::Parent),
                pixels: Arc::new(IconPixels::new(3, 3, vec![255; 36]).unwrap()),
            });
            refresh(&test.window, &presenter.borrow());
            assert_eq!(
                test.window
                    .get_left_rows()
                    .row_data(0)
                    .unwrap()
                    .icon
                    .size()
                    .width,
                3
            );
        }
        for mode in [0, 1, 2] {
            test.window.set_left_view_mode(mode);
            test.draw();
            let row = ui::root_item(&test.window, "..", AccessibleRole::ListItem);
            let artwork = image(row).expect("parent artwork");
            let source = artwork.downcast::<ImageItem>().unwrap();
            let width = source.as_pin_ref().source().size().width;
            assert_eq!(width, if native { 3 } else { 24 }, "mode {mode}");
        }
    }
}
