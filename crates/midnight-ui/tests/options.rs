//! The topic tabs and nested pickers remain usable through actual pointer input.
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::items::AccessibleRole;
use midnight_ui::{NavigationChoiceRow, Theme};
use slint::{ComponentHandle, ModelRc, VecModel, platform::Key};
use std::{cell::Cell, rc::Rc};
use ui::{TestUi, click, click_popup, key};

#[test]
fn options_topics_keep_panel_preferences_and_appearance_readable() {
    let mut ui = TestUi::new();
    let accepted = Rc::new(Cell::new((-1, -1)));
    let changed = accepted.clone();
    let weak = ui.window.as_weak();
    ui.window.on_preferences_changed(move |theme, density, _| {
        changed.set((theme, density));
        weak.upgrade()
            .unwrap()
            .invoke_apply_preferences(theme, density);
    });
    let weak = ui.window.as_weak();
    ui.window.on_search_filter_changed(move |enabled| {
        weak.upgrade().unwrap().set_quick_search_filter(enabled);
    });
    let weak = ui.window.as_weak();
    ui.window.on_sizes_format_changed(move |enabled| {
        weak.upgrade().unwrap().set_sizes_in_bytes(enabled);
    });
    let weak = ui.window.as_weak();
    ui.window
        .on_remember_command_history_changed(move |enabled| {
            weak.upgrade()
                .unwrap()
                .set_remember_command_history(enabled);
        });
    ui.window
        .set_command_history(ModelRc::new(VecModel::from(vec![NavigationChoiceRow {
            identity: "history".into(),
            label: "echo text".into(),
            ..Default::default()
        }])));
    let cleared = Rc::new(Cell::new(0));
    let captured = cleared.clone();
    ui.window
        .on_command_history_clear(move || captured.set(captured.get() + 1));
    ui.window.invoke_show_options();
    ui.draw();
    assert_eq!(ui.window.get_options_topic_index(), 0);
    click(
        &ui.window,
        ui.window.get_options_left() + 35.0,
        ui.window.get_options_top() + 25.0,
    );
    click_popup(
        &ui.window,
        "Show only filename search matches",
        AccessibleRole::Checkbox,
    );
    assert!(
        ui.window.get_quick_search_filter(),
        "Panel search checkbox must work"
    );
    click_popup(
        &ui.window,
        "Always show file sizes in bytes",
        AccessibleRole::Checkbox,
    );
    assert!(
        ui.window.get_sizes_in_bytes(),
        "Byte-size checkbox must work"
    );
    ui.options_topic(1);
    click_popup(&ui.window, "Theme", AccessibleRole::Combobox);
    key(&ui.window, Key::DownArrow);
    key(&ui.window, Key::Return);
    ui.draw();
    assert_eq!(
        accepted.get(),
        (1, 0),
        "The nested theme picker selects Dark"
    );
    click_popup(&ui.window, "Spacing", AccessibleRole::Combobox);
    key(&ui.window, Key::DownArrow);
    key(&ui.window, Key::Return);
    ui.draw();
    assert_eq!(
        accepted.get(),
        (1, 1),
        "Touch applies without closing Options"
    );
    ui.options_topic(0);
    assert!(ui.window.get_quick_search_filter());
    assert!(ui.window.get_sizes_in_bytes());
    key(&ui.window, Key::RightArrow);
    key(&ui.window, Key::RightArrow);
    assert_eq!(
        ui.window.get_options_topic_index(),
        2,
        "repeated arrows reach Command"
    );
    ui.draw();
    click_popup(
        &ui.window,
        "Remember command history after restart",
        AccessibleRole::Checkbox,
    );
    assert!(!ui.window.get_remember_command_history());
    click_popup(&ui.window, "Clear command history", AccessibleRole::Button);
    assert_eq!(cleared.get(), 1);
    ui.options_topic(2);
    key(&ui.window, Key::LeftArrow);
    key(&ui.window, Key::LeftArrow);
    assert_eq!(
        ui.window.get_options_topic_index(),
        0,
        "repeated arrows return to Panel"
    );
    ui.draw();
    ui.options_topic(1);
    let theme = ui.window.global::<Theme>();
    let interior = (
        ui.window.get_options_left() + 5.0,
        ui.window.get_options_top() + 80.0,
    );
    let surface = theme.get_surface();
    let expected = slint::Rgb8Pixel {
        r: surface.red(),
        g: surface.green(),
        b: surface.blue(),
    };
    assert_eq!(ui.pixel(interior.0, interior.1), expected);
    click_popup(&ui.window, "Done", AccessibleRole::Button);
    ui.draw();
    assert!(
        !ui::has_popup(&ui.window),
        "Done closes Options after changing topics and density"
    );
    ui.window.hide().unwrap();
}
