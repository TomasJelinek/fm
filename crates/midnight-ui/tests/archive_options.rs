#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{
    item_tree::ItemRc,
    items::AccessibleRole,
    window::{PopupWindowLocation, WindowInner, WindowKind},
};
use midnight_core::{ARCHIVE_ASSOCIATIONS, Command, Event};
use midnight_ui::{ArchiveOptionsView, Presenter};
use slint::{ComponentHandle, Model};
use std::{cell::RefCell, rc::Rc};
use ui::{TestUi, click_popup};

fn settle(ui: &mut TestUi) {
    // Slint deliberately updates popup placement on the next event-loop tick.
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        ui.draw();
    }
}

fn assert_options_inside_window(ui: &TestUi) {
    let inner = WindowInner::from_pub(ui.window.window());
    let (item, origin) = {
        let popups = inner.active_popups();
        let popup = popups
            .iter()
            .find(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
            .unwrap();
        let PopupWindowLocation::ChildWindow(origin) = popup.location else {
            panic!("software popup must be embedded")
        };
        (ItemRc::new_root(popup.component.clone()), origin)
    };
    let size = item.geometry().size;
    assert!(
        origin.x >= 0.0 && origin.y >= 0.0,
        "popup origin {origin:?}"
    );
    assert!(
        origin.x + size.width <= ui.width as f32 + 0.5,
        "popup right {} exceeds {}",
        origin.x + size.width,
        ui.width
    );
    assert!(
        origin.y + size.height <= ui.height as f32 + 0.5,
        "popup bottom {} exceeds {}",
        origin.y + size.height,
        ui.height
    );
    let done = ui::popup_item(&ui.window, "Done", AccessibleRole::Button);
    let rect = done.geometry();
    let origin = done.map_to_native_window(rect.origin);
    assert!(origin.x >= 0.0 && origin.y >= 0.0);
    assert!(origin.x + rect.size.width <= ui.width as f32 + 0.5);
    assert!(
        origin.y + rect.size.height <= ui.height as f32 + 0.5,
        "Done origin {origin:?}, size {:?}, window {}x{}",
        rect.size,
        ui.width,
        ui.height
    );
}

#[test]
fn archives_topic_has_individual_archive_and_document_checkboxes() {
    let mut ui = TestUi::new();
    let presenter = Rc::new(RefCell::new(Presenter::new("left".into(), "right".into())));
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::default())));
    let saved = Rc::new(RefCell::new(Vec::new()));
    let captured = saved.clone();
    midnight_ui::install(
        &ui.window,
        presenter,
        Rc::new(move |command| {
            if let Command::SavePreferences { preferences, .. } = command {
                captured.borrow_mut().push(preferences);
            }
            true
        }),
    );
    ui.window.invoke_show_options();
    ui.draw();
    click_popup(&ui.window, "Archives", AccessibleRole::Button);
    ui.draw();
    assert_eq!(ui.window.get_options_topic_index(), 7);
    ui::popup_item(&ui.window, ".zip · ZIP archive", AccessibleRole::Checkbox);
    render::snapshot(&ui, "archives-default");
    let window = ui.window.clone_strong();
    let view = window.global::<ArchiveOptionsView>();
    assert_eq!(view.get_rows().row_count(), ARCHIVE_ASSOCIATIONS.len());
    click_popup(&ui.window, ".zip · ZIP archive", AccessibleRole::Checkbox);
    assert!(
        !saved
            .borrow()
            .last()
            .unwrap()
            .archive_association_enabled("zip")
    );
    view.invoke_search("DOCX".into());
    ui.draw();
    assert_eq!(view.get_rows().row_count(), 1);
    assert!(!view.get_rows().row_data(0).unwrap().enabled);
    render::snapshot(&ui, "archives-document-filter");
    for enabled in [true, false] {
        click_popup(
            &ui.window,
            ".docx · Word document / template",
            AccessibleRole::Checkbox,
        );
        ui.draw();
        assert_eq!(view.get_rows().row_data(0).unwrap().enabled, enabled);
        assert_eq!(
            saved
                .borrow()
                .last()
                .unwrap()
                .archive_association_enabled("docx"),
            enabled
        );
    }
    view.invoke_search("".into());
    assert_eq!(view.get_rows().row_count(), ARCHIVE_ASSOCIATIONS.len());
    ui.window.invoke_font_options_changed(0, 32, 15, 1);
    ui.window.set_options_topic_index(0);
    settle(&mut ui);
    click_popup(&ui.window, "Archives", AccessibleRole::Button);
    settle(&mut ui);
    assert_eq!(ui.window.get_options_topic_index(), 7);
    assert_options_inside_window(&ui);
    render::snapshot(&ui, "archives-large-font");
    click_popup(&ui.window, "Done", AccessibleRole::Button);
    assert!(!ui::has_popup(&ui.window));
    ui.window.invoke_show_options();
    settle(&mut ui);
    assert_options_inside_window(&ui);
    ui.resize(1040, 600);
    settle(&mut ui);
    assert_options_inside_window(&ui);
    ui.window.set_options_topic_index(0);
    settle(&mut ui);
    click_popup(&ui.window, "Panel", AccessibleRole::Button);
    for _ in 0..7 {
        ui::key(&ui.window, slint::platform::Key::DownArrow);
    }
    settle(&mut ui);
    assert_eq!(ui.window.get_options_topic_index(), 7);
    render::snapshot(&ui, "archives-large-font-small-window");
    ui.window.invoke_preferences_changed(0, 1, 1);
    settle(&mut ui);
    assert_options_inside_window(&ui);
    click_popup(&ui.window, "Done", AccessibleRole::Button);
    assert!(!ui::has_popup(&ui.window));
}
