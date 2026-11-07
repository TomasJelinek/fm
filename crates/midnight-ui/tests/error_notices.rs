//! Real message-window layout, clipboard dispatch and focus behavior.
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{item_tree::ItemRc, items::AccessibleRole, window::WindowInner};
use midnight_core::*;
use midnight_ui::{Presenter, install, refresh};
use slint::{ComponentHandle, platform::Key};
use std::{cell::RefCell, io, path::PathBuf, rc::Rc, sync::Arc};

#[test]
fn failed_action_is_readable_copyable_and_escape_closes_without_losing_panel() {
    let mut ui = ui::TestUi::new();
    let root = PathBuf::from(std::path::MAIN_SEPARATOR_STR).join("errors");
    let path = root
        .join("資料")
        .join("long-name-".repeat(40))
        .join("žluťoučký 🦀.txt");
    let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root)));
    let commands = Rc::new(RefCell::new(Vec::new()));
    let sink = commands.clone();
    install(
        &ui.window,
        presenter.clone(),
        Rc::new(move |command| {
            sink.borrow_mut().push(command);
            true
        }),
    );
    let navigation = commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::Navigate {
                pane: PaneId::Left,
                navigation,
                ..
            } => Some(*navigation),
            _ => None,
        })
        .unwrap();
    let entries: Arc<[Entry]> = vec![Entry::new(EntryId(1), path.clone(), EntryKind::File)].into();
    presenter.borrow_mut().apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation,
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
    });
    refresh(&ui.window, &presenter.borrow());
    ui.window.invoke_open(0, 1);
    let request = commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::OpenEntry { request, .. } => Some(*request),
            _ => None,
        })
        .unwrap();
    presenter.borrow_mut().apply(Event::DesktopActionFinished {
        request,
        result: Err(FsError::from_io(
            "open file",
            &path,
            io::Error::other("Access denied\nThe application could not open this file."),
        )),
    });
    refresh(&ui.window, &presenter.borrow());
    ui.draw();
    let body = ui.window.get_message_body().to_string();
    assert!(
        body.contains(&format!("\nPath\n{}\n", path.display())),
        "{body}"
    );
    assert!(ui.window.get_message_error());
    ui::popup_item(&ui.window, "Error", AccessibleRole::Text);
    for (scheme, density, width, height) in [(2, 0, 1340, 820), (1, 1, 1040, 650)] {
        ui.window.set_theme_index(scheme);
        ui.window.set_density_index(density);
        ui.window.invoke_apply_preferences(scheme, density);
        slint::platform::update_timers_and_animations();
        ui.resize(width, height);
        let inner = WindowInner::from_pub(ui.window.window());
        let popups = inner.active_popups();
        let popup = popups.last().unwrap();
        let texts = ui::text_values(ItemRc::new_root(popup.component.clone()));
        assert!(texts.iter().any(|text| text.as_str() == body));
        drop(popups);
        for label in ["Copy details", "Close"] {
            let item = ui::popup_item(&ui.window, label, AccessibleRole::Button);
            let geometry = item.geometry();
            let position = item.map_to_native_window(geometry.origin);
            assert!(position.y >= 0.0 && position.y + geometry.height() <= height as f32);
            assert!(position.x >= 0.0 && position.x + geometry.width() <= width as f32);
        }
        render::snapshot(&ui, &format!("error-message-{scheme}-{density}"));
    }
    ui::click_popup(&ui.window, "Copy details", AccessibleRole::Button);
    assert!(
        commands
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::CopyText { text, .. }
        if text == &format!("Desktop action failed\n\n{body}")))
    );
    assert!(ui::has_popup(&ui.window));
    ui::key(&ui.window, Key::Escape);
    ui.draw();
    assert!(!ui::has_popup(&ui.window));
    ui.window.hide().unwrap();
}

#[test]
fn information_dialog_has_no_error_glyph() {
    let mut ui = ui::TestUi::new();
    ui.window.set_message_title("Drive information".into());
    ui.window
        .set_message_body("Path\nC:\\\n\nAvailable: 100 GB".into());
    ui.window.set_message_error(false);
    ui.window.set_message_generation(1);
    slint::platform::update_timers_and_animations();
    ui.draw();
    let inner = WindowInner::from_pub(ui.window.window());
    let popups = inner.active_popups();
    let texts = ui::text_values(ItemRc::new_root(popups.last().unwrap().component.clone()));
    assert!(!texts.iter().any(|text| text.as_str() == "!"));
    drop(popups);
    ui::key(&ui.window, Key::Escape);
    ui.draw();
    assert!(!ui::has_popup(&ui.window));
    ui.window.hide().unwrap();
}

#[test]
fn panel_error_has_a_readable_multiline_body_at_the_minimum_window_size() {
    let mut ui = ui::TestUi::new();
    ui.window.set_theme_index(1);
    ui.window.invoke_apply_preferences(1, 0);
    let message = "Access denied\n\nPath\nC:\\資料\\protected\\folder\n\nOperation: list directory";
    ui.window.set_left_error(message.into());
    slint::platform::update_timers_and_animations();
    ui.resize(1040, 480);
    ui::root_item(&ui.window, "Error", AccessibleRole::Text);
    assert!(ui.window.get_left_list_height() >= 22.0);
    render::snapshot(&ui, "error-panel-minimum");
    ui.window.hide().unwrap();
}
