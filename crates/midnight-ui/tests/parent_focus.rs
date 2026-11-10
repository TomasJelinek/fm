//! Parent-route and viewport checks in a software window; no OS desktop automation.
use i_slint_core::{
    accessibility::AccessibleStringProperty, item_tree::ItemRc, items::AccessibleRole,
    window::WindowInner,
};
use midnight_core::*;
use midnight_ui::{Presenter, Theme, install, refresh};
use slint::{ComponentHandle, Model, platform::Key};
use std::{cell::RefCell, path::Path, rc::Rc, sync::Arc};
#[path = "support/ui.rs"]
mod ui;

fn parent_button(item: ItemRc) -> Option<ItemRc> {
    if item.accessible_role() == AccessibleRole::Button
        && item
            .accessible_string_property(AccessibleStringProperty::Label)
            .is_some_and(|label| label.starts_with("Parent folder"))
    {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = parent_button(item) {
            return Some(found);
        }
    }
    None
}

fn navigation(commands: &RefCell<Vec<Command>>) -> NavigationId {
    commands
        .borrow()
        .iter()
        .rev()
        .find_map(|command| match command {
            Command::Navigate {
                navigation,
                pane: PaneId::Left,
                ..
            }
            | Command::Refresh {
                navigation,
                pane: PaneId::Left,
                ..
            } => Some(*navigation),
            _ => None,
        })
        .unwrap()
}

fn finish(p: &Rc<RefCell<Presenter>>, commands: &RefCell<Vec<Command>>, entries: Vec<Entry>) {
    let navigation = navigation(commands);
    let index = Arc::new(DirectoryIndex::from_entries(&entries));
    p.borrow_mut().apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation,
        entries: entries.into(),
        index,
    });
}

fn draw_first_backend_frame(ui: &mut ui::TestUi) {
    // Winit updates timers/change handlers in new_events, before delivering
    // backend user events. RedrawRequested then renders without a second sweep.
    // TestUi::draw would grant deferred cursor correction an extra event-loop turn.
    ui.native.draw_if_needed(|renderer| {
        renderer.render(&mut ui.pixels, ui.width);
    });
}

fn assert_child_visible(ui: &ui::TestUi, child: &Path, route: usize) {
    assert_eq!(ui.window.get_left_cursor(), 151, "route {route}");
    assert!(ui.window.get_left_visible_row_start() <= 151);
    assert!(
        ui.window.get_left_visible_row_end() > 151,
        "child must be visible: route={route}, range={}..{}, y={}, height={}",
        ui.window.get_left_visible_row_start(),
        ui.window.get_left_visible_row_end(),
        ui.window.get_left_list_content_y(),
        ui.window.get_left_list_visible_height(),
    );
    assert!(!ui.window.get_left_rows().row_data(151).unwrap().selected);
    let item = ui::root_item(
        &ui.window,
        child.file_name().unwrap().to_str().unwrap(),
        AccessibleRole::ListItem,
    );
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    let top = ui.window.get_left_list_top();
    assert!(origin.y >= top - 0.5, "route {route}: row above viewport");
    assert!(
        origin.y + rect.size.height <= top + ui.window.get_left_list_visible_height() + 0.5,
        "route {route}: row below viewport: origin={origin:?}, rect={rect:?}, top={top}, height={}, y={}, range={}..{}",
        ui.window.get_left_list_visible_height(),
        ui.window.get_left_list_content_y(),
        ui.window.get_left_visible_row_start(),
        ui.window.get_left_visible_row_end(),
    );
    let edge = ui.window.global::<Theme>().get_cursor_edge();
    assert_eq!(
        ui.pixel(origin.x + 1.0, origin.y + rect.size.height / 2.0),
        slint::Rgb8Pixel {
            r: edge.red(),
            g: edge.green(),
            b: edge.blue()
        },
        "route {route}: returned child must be visibly highlighted in the first rendered frame",
    );
}

fn entries(parent: &Path, child: &Path) -> Vec<Entry> {
    let mut rows: Vec<_> = (0..150)
        .map(|i| {
            Entry::new(
                EntryId(i + 1),
                parent.join(format!("folder-{i:03}")),
                EntryKind::Directory,
            )
        })
        .collect();
    rows.push(Entry::new(
        EntryId(200),
        child.to_owned(),
        EntryKind::Directory,
    ));
    rows
}

#[test]
fn parent_routes_focus_child_and_scroll_it_into_view_without_checking_it() {
    let mut ui = ui::TestUi::new();
    let parent = std::env::temp_dir().join("parent 文档");
    let child = parent.join("Žluťoučký child");
    let p = Rc::new(RefCell::new(Presenter::new(child.clone(), parent.clone())));
    let commands = Rc::new(RefCell::new(Vec::new()));
    let sent = commands.clone();
    install(
        &ui.window,
        p.clone(),
        Rc::new(move |command| {
            sent.borrow_mut().push(command);
            true
        }),
    );
    for route in 0..4 {
        if route != 0 {
            ui.window
                .invoke_navigate(0, child.to_string_lossy().as_ref().into());
        }
        finish(&p, &commands, vec![]);
        ui.window.invoke_view_mode(
            0,
            if route == 1 {
                1
            } else if route == 2 {
                2
            } else {
                0
            },
        );
        refresh(&ui.window, &p.borrow());
        ui.draw();
        ui.window.invoke_focus_files();
        match route {
            0 => ui::key(&ui.window, Key::Backspace),
            1 => ui::key(&ui.window, Key::Return),
            2 => {
                let position = ui::root_position(&ui.window, "..", AccessibleRole::ListItem);
                ui::click(&ui.window, position.x, position.y);
                ui::click(&ui.window, position.x, position.y);
            }
            _ => {
                let root = WindowInner::from_pub(ui.window.window()).component();
                let button = parent_button(ItemRc::new_root(root)).unwrap();
                let rect = button.geometry();
                let origin = button.map_to_native_window(rect.origin);
                ui::click(
                    &ui.window,
                    origin.x + rect.size.width / 2.0,
                    origin.y + rect.size.height / 2.0,
                );
            }
        }
        assert_eq!(
            commands
                .borrow()
                .iter()
                .rev()
                .find_map(|command| match command {
                    Command::Navigate { path, .. } => Some(path.clone()),
                    _ => None,
                })
                .as_ref(),
            Some(&parent),
            "route {route} must navigate to parent"
        );
        if route == 0 {
            // A progressive batch already containing the returned child must
            // reveal it on its first frame too, before the final sorted snapshot.
            slint::platform::update_timers_and_animations();
            p.borrow_mut().apply(Event::EntriesBatch {
                pane: PaneId::Left,
                navigation: navigation(&commands),
                entries: entries(&parent, &child),
            });
            refresh(&ui.window, &p.borrow());
            draw_first_backend_frame(&mut ui);
            assert_child_visible(&ui, &child, route);
        }
        slint::platform::update_timers_and_animations();
        finish(&p, &commands, entries(&parent, &child));
        refresh(&ui.window, &p.borrow());
        draw_first_backend_frame(&mut ui);
        // The very first frame of the parent listing must already reveal the
        // returned child. Waiting for another layout frame hides a visible jump.
        assert_child_visible(&ui, &child, route);
        let item = ui::root_item(
            &ui.window,
            child.file_name().unwrap().to_str().unwrap(),
            AccessibleRole::ListItem,
        );
        let rect = item.geometry();
        // Selection autoscroll is deliberate viewport input. Later geometry
        // changes must not revive the last automatic cursor reveal.
        ui.window.invoke_scroll_selection(0, rect.size.height * 4.0);
        ui.draw();
        let scrolled_y = ui.window.get_left_list_content_y();
        ui.window.invoke_action(0, 3);
        let mut refreshed = entries(&parent, &child);
        refreshed.push(Entry::new(
            EntryId(300),
            parent.join("another folder"),
            EntryKind::Directory,
        ));
        finish(&p, &commands, refreshed);
        refresh(&ui.window, &p.borrow());
        ui.draw();
        // The SDK may normalize a model reset by a fractional row. A four-row
        // user scroll must survive within that bound, without returning to the cursor.
        assert!(
            (ui.window.get_left_list_content_y() - scrolled_y).abs() <= rect.size.height + 0.5,
            "route {route}: geometry revived a canceled reveal"
        );
    }
}
