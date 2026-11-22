//! The topic tabs and nested pickers remain usable through actual pointer input.
#[path = "support/popup_scroll.rs"]
mod popup_scroll;
use crate::support::ui;

use i_slint_core::{accessibility::AccessibleStringProperty, items::AccessibleRole};
use midnight_ui::{NavigationChoiceRow, Theme};
use slint::{ComponentHandle, ModelRc, VecModel, platform::Key};
use std::{cell::Cell, rc::Rc};
use ui::{TestUi, click, click_popup, key};

/// The Options pages in their listed order with their topic identities.
const PAGES: [(&str, i32); 10] = [
    ("General", 0),
    ("Appearance", 1),
    ("Colors", 8),
    ("Previews", 6),
    ("Archives", 7),
    ("Editors", 4),
    ("Comparators", 10),
    ("Custom Commands", 9),
    ("Keyboard", 3),
    ("Plugins", 5),
];

/// A control that only the page of this topic shows, with its role.
fn page_control(topic: i32) -> (&'static str, AccessibleRole) {
    match topic {
        0 => ("Show hidden files", AccessibleRole::Checkbox),
        1 => ("Theme", AccessibleRole::Combobox),
        3 => ("Search commands", AccessibleRole::TextInput),
        4 => ("Editor rules", AccessibleRole::List),
        5 => ("Rescan all plugins", AccessibleRole::Button),
        6 => ("Show image and video previews", AccessibleRole::Checkbox),
        7 => ("Search archive associations", AccessibleRole::TextInput),
        8 => ("Filename color rules", AccessibleRole::List),
        9 => ("Configured Custom Commands", AccessibleRole::List),
        10 => ("Comparator rules", AccessibleRole::List),
        _ => unreachable!("no Options topic {topic}"),
    }
}

#[test]
fn options_pages_list_in_reading_order_and_keep_their_topic_identities() {
    assert_eq!(PAGES.map(|(_, topic)| topic), ui::OPTIONS_TOPIC_ORDER);
    let mut ui = TestUi::new();
    ui.window.invoke_show_options();
    ui.draw();
    let tops = PAGES.map(|(title, _)| {
        let button = ui::popup_item(&ui.window, title, AccessibleRole::Button);
        button.map_to_native_window(button.geometry().origin).y
    });
    assert!(
        tops.windows(2).all(|pair| pair[0] < pair[1]),
        "the list shows the pages top to bottom in their order: {tops:?}"
    );
    // A click shows the page's own content under its own topic identity.
    for (title, topic) in PAGES {
        ui.options_topic(topic);
        let (control, role) = page_control(topic);
        assert!(
            ui::try_popup_item(&ui.window, control, role).is_some(),
            "{title} shows {control}"
        );
    }
    // Arrows walk the list in the same order and stop at its ends.
    ui.options_topic(0);
    for (title, topic) in PAGES.iter().skip(1).chain(PAGES.last()) {
        key(&ui.window, Key::DownArrow);
        ui.draw();
        let shown = ui.window.get_options_topic_index();
        assert_eq!(shown, *topic, "Down to {title}");
    }
    for (title, topic) in PAGES.iter().rev().skip(1).chain(PAGES.first()) {
        key(&ui.window, Key::UpArrow);
        ui.draw();
        let shown = ui.window.get_options_topic_index();
        assert_eq!(shown, *topic, "Up to {title}");
    }
    // A page opened by its identity (the Custom Commands shortcut, a page's validation
    // error, the page shown when Options closed) is the one marked in the list.
    for (title, topic) in PAGES {
        ui.window.set_options_topic_index(topic);
        ui.draw();
        let marked: Vec<&str> = PAGES
            .iter()
            .map(|(other, _)| *other)
            .filter(|other| {
                ui::popup_item(&ui.window, other, AccessibleRole::Button)
                    .accessible_string_property(AccessibleStringProperty::Checked)
                    .is_some_and(|checked| checked == "true")
            })
            .collect();
        assert_eq!(marked, [title], "topic {topic}");
    }
    // A narrow window's topic picker lists the pages in the same order.
    ui.resize(600, 700);
    for (title, topic) in PAGES {
        ui.options_topic(topic);
        let picker = ui::popup_item(&ui.window, "Options topic", AccessibleRole::Combobox);
        assert_eq!(
            picker
                .accessible_string_property(AccessibleStringProperty::Value)
                .as_deref(),
            Some(title)
        );
    }
}

#[test]
fn options_topics_keep_panel_preferences_and_appearance_readable() {
    let mut ui = TestUi::new();
    ui.window.set_preferences_ready(true);
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
    // Alphabetic display order, independent of the persisted Dark theme ID.
    for _ in 0..4 {
        key(&ui.window, Key::DownArrow);
    }
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
    // Command history belongs to General; reveal its controls in the touch layout.
    popup_scroll::click(
        &mut ui,
        "Remember command history after restart",
        AccessibleRole::Checkbox,
    );
    assert!(!ui.window.get_remember_command_history());
    popup_scroll::click(&mut ui, "Clear command history", AccessibleRole::Button);
    assert_eq!(cleared.get(), 1);
    ui.options_topic(0);
    key(&ui.window, Key::RightArrow);
    key(&ui.window, Key::RightArrow);
    assert_eq!(
        ui.window.get_options_topic_index(),
        8,
        "repeated arrows reach Colors, the third page, without a Command page"
    );
    ui.draw();
    key(&ui.window, Key::LeftArrow);
    key(&ui.window, Key::LeftArrow);
    assert_eq!(
        ui.window.get_options_topic_index(),
        0,
        "repeated arrows return to General"
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
    click_popup(&ui.window, "Cancel", AccessibleRole::Button);
    ui.draw();
    assert!(
        !ui::has_popup(&ui.window),
        "Cancel closes Options after changing topics and density"
    );
    ui.window.hide().unwrap();
}
