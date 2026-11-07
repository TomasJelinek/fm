//! The command editor and its controls share the footer's compact row geometry.
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{item_tree::ItemRc, items::AccessibleRole};
use midnight_ui::{NavigationChoiceRow, Theme};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel, platform::Key};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

fn configure(test: &mut ui::TestUi, path: &str) {
    test.window.set_left_path(path.into());
    test.window
        .set_terminal_choices(ModelRc::new(VecModel::from(vec![
            "PowerShell".into(),
            "cmd".into(),
        ])));
    test.window.set_windows_shells(true);
    test.window.set_terminal_available(true);
    test.window.set_terminal_ready(true);
    test.window
        .set_command_history(ModelRc::new(VecModel::from(vec![NavigationChoiceRow {
            identity: "history-one".into(),
            label: "echo history 文档".into(),
            ..Default::default()
        }])));
    test.window.invoke_set_command("echo draft 文档".into(), 0);
    test.draw();
}

fn in_command_row(item: ItemRc, top: f32, height: f32, label: &str) {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    assert!(
        (rect.height() - height).abs() < 0.1,
        "{label} must match the row height {height}, got {rect:?}"
    );
    assert!(
        origin.y >= top - 0.5 && origin.y + rect.height() <= top + height + 0.5,
        "{label} must fit in command row {top}..{}, got {origin:?} {rect:?}",
        top + height
    );
}

#[test]
fn command_editor_and_controls_match_the_footer_across_themes_and_font_sizes() {
    let mut test = ui::TestUi::new();
    let path = format!("C:\\文档\\{}\\tail", "e\u{301}👩🏽‍💻".repeat(80));
    configure(&mut test, &path);
    for width in [1340, 1040] {
        test.resize(width, 820);
        for scheme in 0..9 {
            for density in 0..2 {
                for font in [8, 11, 15, 32] {
                    test.window.invoke_apply_fonts("".into(), font, font);
                    test.window.invoke_apply_preferences(scheme, density);
                    test.draw();
                    let height = test.window.global::<Theme>().get_bottom_height();
                    let top = test.window.get_command_bar_top();
                    for (label, actual) in [
                        ("command row", test.window.get_command_bar_height()),
                        ("command editor", test.window.get_command_input_height()),
                        ("working folder", test.window.get_command_path_height()),
                    ] {
                        assert!(
                            (actual - height).abs() < 0.1,
                            "{label} must match footer height: width={width}, theme={scheme}, density={density}, font={font}, actual={actual}, footer={height}"
                        );
                    }
                    assert!((test.window.get_command_input_top() - top).abs() < 0.1);
                    assert!((test.window.get_command_path_top() - top).abs() < 0.1);
                    assert_eq!(test.window.get_command_prompt(), format!("{path}>"));
                    assert!(
                        test.window.get_command_path_content_width()
                            > test.window.get_command_path_width()
                    );
                    assert!(test.window.get_command_input_width() >= 300.0);
                    for (label, role) in [
                        ("Command shell".to_owned(), AccessibleRole::Combobox),
                        (
                            "Executed command history".to_owned(),
                            AccessibleRole::Combobox,
                        ),
                        (
                            format!("Open PowerShell\nWorking folder: {path}"),
                            AccessibleRole::Button,
                        ),
                        ("Run".to_owned(), AccessibleRole::Button),
                        ("Command line".to_owned(), AccessibleRole::TextInput),
                    ] {
                        in_command_row(
                            ui::root_item(&test.window, &label, role),
                            top,
                            height,
                            &label,
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn compact_command_row_preserves_shell_history_run_and_keyboard_focus() {
    let mut test = ui::TestUi::new();
    configure(&mut test, "C:\\文档");
    let launched = Rc::new(Cell::new(0));
    let captured = launched.clone();
    test.window
        .on_open_terminal(move || captured.set(captured.get() + 1));
    let weak = test.window.as_weak();
    test.window.on_terminal_changed(move |index| {
        weak.upgrade().unwrap().set_terminal_index(index);
    });
    let submitted = Rc::new(RefCell::new(Vec::new()));
    let captured = submitted.clone();
    test.window.on_submit_command(move |text| {
        captured.borrow_mut().push(text);
        true
    });
    let weak = test.window.as_weak();
    test.window.on_command_history_choice(move |identity| {
        assert_eq!(identity, "history-one");
        let text = SharedString::from("echo history 文档");
        weak.upgrade()
            .unwrap()
            .invoke_set_command(text.clone(), text.len() as i32);
    });
    for (density, font) in [(0, 8), (0, 11), (1, 11), (1, 15), (0, 32), (1, 32)] {
        test.window.invoke_apply_fonts("".into(), font, font);
        test.window.invoke_apply_preferences(0, density);
        test.window.set_terminal_index(0);
        let draft = SharedString::from("echo literal 文档");
        test.window
            .invoke_set_command(draft.clone(), draft.len() as i32);
        test.draw();
        let before = submitted.borrow().len();
        ui::click_root(&test.window, "Command shell", AccessibleRole::Combobox);
        ui::key(&test.window, Key::End);
        ui::key(&test.window, Key::Return);
        assert_eq!(test.window.get_terminal_index(), 1);
        assert_eq!(test.window.get_command_text(), draft);
        assert!(test.window.get_command_focused());
        let before_launch = launched.get();
        ui::click_root(
            &test.window,
            "Open cmd\nWorking folder: C:\\文档",
            AccessibleRole::Button,
        );
        assert_eq!(launched.get(), before_launch + 1);
        assert_eq!(test.window.get_command_text(), draft);
        assert_eq!(submitted.borrow().len(), before);
        assert!(test.window.get_command_focused());
        ui::click_root(
            &test.window,
            "Executed command history",
            AccessibleRole::Combobox,
        );
        ui::key(&test.window, Key::Return);
        assert_eq!(test.window.get_command_text(), "echo history 文档");
        assert_eq!(submitted.borrow().len(), before);
        assert!(test.window.get_command_focused());
        ui::click_root(&test.window, "Run", AccessibleRole::Button);
        assert_eq!(submitted.borrow().last().unwrap(), "echo history 文档");
        assert!(test.window.get_command_text().is_empty());
        assert!(test.window.get_command_focused());
        ui::click_root(&test.window, "Run", AccessibleRole::Button);
        assert_eq!(
            submitted.borrow().len(),
            before + 1,
            "empty Run stays disabled"
        );
        test.window
            .invoke_set_command(draft.clone(), draft.len() as i32);
        ui::key(&test.window, Key::Return);
        assert_eq!(submitted.borrow().last().unwrap(), &draft);
        assert_eq!(submitted.borrow().len(), before + 2);
        ui::key(&test.window, Key::Escape);
        assert_eq!(test.window.get_focused_pane(), 0);
    }
}
