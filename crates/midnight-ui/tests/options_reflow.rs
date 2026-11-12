//! Settings pages fit a narrow viewport; actions remain inside their real clips.
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, Flickable},
};
use midnight_ui::{
    ColorOptionsView, EditorOptionsDraft, EditorOptionsView, KeyboardEditorView, ShortcutCommandRow,
};
use slint::{
    ComponentHandle,
    platform::{Key, WindowEvent},
};
use std::{cell::RefCell, rc::Rc};

fn bounds(item: &ItemRc) -> (f32, f32, f32, f32) {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    (
        origin.x,
        origin.y,
        origin.x + rect.width(),
        origin.y + rect.height(),
    )
}

fn scroll_parent(item: &ItemRc) -> ItemRc {
    let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    loop {
        let item = parent.expect("settings control must have a scrolling viewport");
        if item.downcast::<Flickable>().is_some() {
            return item;
        }
        parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
}

fn inside_clips(item: &ItemRc, width: usize, height: usize) {
    let area = bounds(item);
    assert!(
        area.0 >= 0.0
            && area.1 >= 0.0
            && area.2 <= width as f32 + 1.0
            && area.3 <= height as f32 + 1.0,
        "control outside window: {area:?}"
    );
    let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(item) = parent {
        if item.downcast::<Flickable>().is_some() {
            let clip = bounds(&item);
            assert!(
                area.0 >= clip.0 - 1.0
                    && area.1 >= clip.1 - 1.0
                    && area.2 <= clip.2 + 1.0
                    && area.3 <= clip.3 + 1.0,
                "control {area:?} is clipped by its scrolling ancestor {clip:?}"
            );
        }
        parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
}

#[test]
fn every_options_page_keeps_cancel_inside_its_clips_at_the_minimum_size() {
    let mut test = ui::TestUi::new();
    for (density, font) in [(0, 11), (1, 32)] {
        test.window.invoke_apply_preferences(0, density);
        test.window.invoke_apply_fonts("".into(), font, font);
        test.resize(320, 200);
        test.window.invoke_show_options();
        for topic in [0, 1, 3, 4, 5, 6, 7, 8, 9] {
            test.window.set_options_topic_index(topic);
            test.draw();
            inside_clips(
                &ui::popup_item(&test.window, "Cancel", AccessibleRole::Button),
                320,
                200,
            );
            render::snapshot(
                &test,
                &format!("settings-reflow-{topic}-{density}-{font}-320"),
            );
        }
        ui::key(&test.window, Key::Escape);
    }
}

#[test]
fn narrow_appearance_uses_vertical_scrolling_and_its_theme_picker_remains_interactive() {
    let mut test = ui::TestUi::new();
    let changed = Rc::new(RefCell::new(Vec::new()));
    let captured = changed.clone();
    test.window
        .on_preferences_changed(move |theme, density, field| {
            captured.borrow_mut().push((theme, density, field));
        });
    test.window.invoke_apply_fonts("".into(), 32, 32);
    test.window.invoke_apply_preferences(0, 1);
    test.resize(320, 600);
    test.window.invoke_show_options();
    test.window.set_options_topic_index(1);
    for _ in 0..3 {
        test.draw();
    }
    let picker = ui::popup_item(&test.window, "Theme", AccessibleRole::Combobox);
    let viewport = scroll_parent(&picker);
    let clip = bounds(&viewport);
    let content_width = viewport
        .downcast::<Flickable>()
        .unwrap()
        .as_pin_ref()
        .content_width()
        .get();
    assert!(
        content_width <= clip.2 - clip.0 + 1.0,
        "Appearance must reflow instead of imposing a horizontal canvas: {content_width}, {clip:?}"
    );
    let area = bounds(&picker);
    test.window
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position: slint::LogicalPosition::new(clip.0 + 4.0, clip.1 + 4.0),
            delta_x: 0.0,
            delta_y: (area.1 - clip.1).min(0.0),
        });
    test.draw();
    inside_clips(&picker, 320, 600);
    ui::click_popup(&test.window, "Theme", AccessibleRole::Combobox);
    ui::key(&test.window, Key::DownArrow);
    ui::key(&test.window, Key::Return);
    assert!(
        !changed.borrow().is_empty(),
        "the visible picker must still accept input"
    );
    render::snapshot(&test, "settings-appearance-reflow-320-font32");
}

#[test]
fn editor_resize_preserves_the_field_instance_its_caret_and_the_complete_draft() {
    let mut test = ui::TestUi::new();
    let window = test.window.clone_strong();
    let editor = window.global::<EditorOptionsView>();
    editor.set_draft(EditorOptionsDraft {
        executable: "ABC".into(),
        arguments: "[\"文档 two words\",\"\"]".into(),
        use_default: false,
        ..Default::default()
    });
    test.window.invoke_show_options();
    test.window.set_options_topic_index(4);
    test.draw();
    ui::click_popup(&test.window, "Editor executable", AccessibleRole::TextInput);
    ui::key(&test.window, Key::Home);
    ui::key(&test.window, Key::RightArrow);
    test.resize(320, 600);
    ui::key(&test.window, "X");
    assert_eq!(
        editor.get_draft().executable.as_str(),
        "AXBC",
        "reflow must preserve the caret in the existing field"
    );
    assert_eq!(
        editor.get_draft().arguments.as_str(),
        "[\"文档 two words\",\"\"]"
    );
    test.resize(1340, 820);
    ui::key(&test.window, "Y");
    assert_eq!(editor.get_draft().executable.as_str(), "AXYBC");
}

#[test]
fn argument_lists_reserve_space_for_their_actual_rows_up_to_a_practical_cap() {
    let mut test = ui::TestUi::new();
    let window = test.window.clone_strong();
    let editor = window.global::<EditorOptionsView>();
    editor.set_draft(EditorOptionsDraft {
        use_default: false,
        ..Default::default()
    });
    test.window.invoke_show_options();
    test.window.set_options_topic_index(4);
    for count in [1, 2, 20] {
        editor.set_argument_rows(slint::ModelRc::new(slint::VecModel::from(
            (0..count)
                .map(|index| format!("Argument {index}").into())
                .collect::<Vec<slint::SharedString>>(),
        )));
        for _ in 0..3 {
            test.draw();
        }
        let row = ui::popup_item(&test.window, "Editor argument 1", AccessibleRole::ListItem);
        let viewport = scroll_parent(&row);
        assert!(
            viewport.geometry().height() <= row.geometry().height() * count.min(4) as f32 + 4.0,
            "a short argument list must not reserve unused rows"
        );
        render::snapshot(&test, &format!("settings-arguments-{count}-rows"));
    }
}

#[test]
fn a_single_keyboard_search_result_has_a_complete_row_inside_its_border() {
    let mut test = ui::TestUi::new();
    let window = test.window.clone_strong();
    let keyboard = window.global::<KeyboardEditorView>();
    keyboard.set_commands(slint::ModelRc::new(slint::VecModel::from(vec![
        ShortcutCommandRow {
            id: "files.rename".into(),
            label: "Only matching command".into(),
            selected: true,
            ..Default::default()
        },
    ])));
    test.window.invoke_show_options();
    test.window.set_options_topic_index(3);
    for _ in 0..3 {
        test.draw();
    }
    let row = ui::popup_item(
        &test.window,
        "Only matching command",
        AccessibleRole::ListItem,
    );
    let viewport = scroll_parent(&row);
    assert!(
        viewport.geometry().height() >= row.geometry().height(),
        "the catalog border must not subtract space from the only result row"
    );
}

#[test]
fn increasing_the_font_keeps_the_selected_keyboard_command_inside_the_catalog() {
    let mut test = ui::TestUi::new();
    let window = test.window.clone_strong();
    let keyboard = window.global::<KeyboardEditorView>();
    keyboard.set_commands(slint::ModelRc::new(slint::VecModel::from(
        (0..20)
            .map(|index| ShortcutCommandRow {
                id: format!("command-{index}").into(),
                label: format!("Command {index}").into(),
                selected: index == 7,
                ..Default::default()
            })
            .collect::<Vec<_>>(),
    )));
    test.window.invoke_show_options();
    test.window.set_options_topic_index(3);
    test.draw();
    keyboard.set_selected_row(7);
    for _ in 0..3 {
        test.draw();
    }
    test.window.invoke_apply_fonts("".into(), 32, 32);
    for _ in 0..3 {
        test.draw();
    }
    let row = ui::popup_item(&test.window, "Command 7", AccessibleRole::ListItem);
    let area = bounds(&row);
    let viewport = bounds(&scroll_parent(&row));
    assert!(
        area.1 >= viewport.1 - 1.0 && area.3 <= viewport.3 + 1.0,
        "font preview must keep the selected command visible: {area:?}, {viewport:?}"
    );
}

#[test]
fn empty_color_rules_show_the_whole_wrapped_instruction_at_large_font() {
    let mut test = ui::TestUi::new();
    test.window
        .global::<ColorOptionsView>()
        .set_labels(Default::default());
    test.window.invoke_apply_fonts("".into(), 32, 32);
    test.resize(320, 600);
    test.window.invoke_show_options();
    test.window.set_options_topic_index(8);
    for _ in 0..3 {
        test.draw();
    }
    let message = ui::popup_item(
        &test.window,
        "No rules. Add a rule to choose filename colors.",
        AccessibleRole::Text,
    );
    assert!(
        message.geometry().height() >= 64.0,
        "the empty instruction wraps to multiple lines and cannot fit one control row"
    );
    render::snapshot(&test, "settings-empty-colors-320-font32");
}

#[test]
fn wrapping_checkbox_preserves_a_full_touch_target_and_keyboard_toggle() {
    let mut test = ui::TestUi::new();
    let toggles = Rc::new(RefCell::new(Vec::new()));
    let captured = toggles.clone();
    test.window
        .on_sizes_format_changed(move |checked| captured.borrow_mut().push(checked));
    test.window.invoke_apply_preferences(0, 1);
    test.window.invoke_show_options();
    test.window.set_options_topic_index(0);
    for _ in 0..3 {
        test.draw();
    }
    let checkbox = ui::popup_item(
        &test.window,
        "Always show file sizes in bytes",
        AccessibleRole::Checkbox,
    );
    assert!(
        checkbox.geometry().height() >= 40.0,
        "the native checkbox hit target must fill the touch row"
    );
    let area = bounds(&checkbox);
    ui::click(&test.window, (area.0 + area.2) / 2.0, area.3 - 2.0);
    assert_eq!(toggles.borrow().as_slice(), &[true]);
    ui::key(&test.window, Key::Space);
    assert_eq!(toggles.borrow().as_slice(), &[true, false]);
}
