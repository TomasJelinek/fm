//! Real settings callbacks preserve literal argv while list selection follows edits.
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{
    accessibility::{AccessibilityAction, AccessibleStringProperty},
    items::AccessibleRole,
};
use midnight_core::*;
use midnight_ui::{EditorOptionsView, Presenter, UserCommandSettingsView, install, refresh};
use slint::{ComponentHandle, Model, platform::Key};
use std::{cell::RefCell, rc::Rc, sync::Arc};

fn values(test: &ui::TestUi, command: bool) -> Vec<String> {
    let model = if command {
        test.window
            .global::<UserCommandSettingsView>()
            .get_argument_rows()
    } else {
        test.window
            .global::<EditorOptionsView>()
            .get_argument_rows()
    };
    model.iter().map(|value| value.to_string()).collect()
}

fn exercise(command: bool) {
    let mut test = ui::TestUi::new();
    let executable = std::env::current_exe().unwrap();
    let placeholder = if command { "${file.path}" } else { "{file}" };
    let original = vec![
        placeholder.to_owned(),
        String::new(),
        "文档 two words".into(),
    ];
    let preferences = UserPreferences {
        editor: EditorPreferences {
            executable: Some(executable.clone()),
            arguments: original.clone(),
        },
        user_commands: Arc::new(vec![UserCommand {
            id: UserCommandId::new("argument-list").unwrap(),
            name: "Argument list".into(),
            enabled: true,
            toolbar_label: None,
            icon: None,
            working_directory: "${active.path}".into(),
            launch: UserCommandLaunch::Single,
            invocation: UserCommandInvocation::Program {
                executable,
                arguments: original,
            },
        }]),
        ..Default::default()
    };
    let presenter = Rc::new(RefCell::new(Presenter::new("left".into(), "right".into())));
    let sent = Rc::new(RefCell::new(Vec::new()));
    let commands = sent.clone();
    install(
        &test.window,
        presenter.clone(),
        Rc::new(move |command| {
            commands.borrow_mut().push(command);
            true
        }),
    );
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::new(preferences))));
    refresh(&test.window, &presenter.borrow());
    test.window.invoke_show_options();
    test.window
        .set_options_topic_index(if command { 9 } else { 4 });
    test.draw();
    let prefix = if command { "Command" } else { "Editor" };
    let label = |index| format!("{prefix} argument {index}");
    let select = |test: &mut ui::TestUi, index| {
        ui::popup_item(&test.window, &label(index), AccessibleRole::ListItem)
            .accessible_action(&AccessibilityAction::Default);
        test.draw();
    };
    let selected = |test: &ui::TestUi, index| {
        assert_eq!(
            ui::popup_item(&test.window, &label(index), AccessibleRole::ListItem)
                .accessible_string_property(AccessibleStringProperty::ItemSelected)
                .as_deref(),
            Some("true")
        );
    };
    let editor_value = |test: &ui::TestUi, index, expected: &str| {
        assert_eq!(
            ui::popup_item(&test.window, &label(index), AccessibleRole::TextInput)
                .accessible_string_property(AccessibleStringProperty::Value)
                .as_deref(),
            Some(expected),
            "the editable value must follow the selected argument"
        );
    };
    let action = |test: &mut ui::TestUi, name| {
        ui::popup_item(&test.window, name, AccessibleRole::Button)
            .accessible_action(&AccessibilityAction::Default);
        test.draw();
    };
    let row = ui::popup_item(&test.window, &label(1), AccessibleRole::ListItem);
    let row_top = row.map_to_native_window(row.geometry().origin).y;
    for name in [
        "Add argument",
        "Remove argument",
        "Move argument up",
        "Move argument down",
    ] {
        let button = ui::popup_item(&test.window, name, AccessibleRole::Button);
        let rect = button.geometry();
        assert!(button.map_to_native_window(rect.origin).y + rect.height() <= row_top);
        assert!(rect.width() <= 60.0);
    }
    select(&mut test, 2);
    action(&mut test, "Move argument up");
    assert_eq!(values(&test, command), ["", placeholder, "文档 two words"]);
    selected(&test, 1);
    editor_value(&test, 1, "");
    ui::popup_item(&test.window, &label(1), AccessibleRole::TextInput).accessible_action(
        &AccessibilityAction::SetValue(" \"literal 文档\" & ".into()),
    );
    test.draw();
    assert_eq!(
        values(&test, command),
        [" \"literal 文档\" & ", placeholder, "文档 two words"]
    );
    action(&mut test, "Add argument");
    selected(&test, 4);
    editor_value(&test, 4, "");
    assert_eq!(values(&test, command).last().unwrap(), "");
    action(&mut test, "Move argument up");
    selected(&test, 3);
    editor_value(&test, 3, "");
    action(&mut test, "Remove argument");
    editor_value(&test, 3, "文档 two words");
    assert_eq!(
        values(&test, command),
        [" \"literal 文档\" & ", placeholder, "文档 two words"]
    );
    action(
        &mut test,
        if command {
            "Insert argument value"
        } else {
            "Insert editor file argument"
        },
    );
    selected(&test, 4);
    editor_value(&test, 4, placeholder);
    assert_eq!(
        values(&test, command),
        [
            " \"literal 文档\" & ",
            placeholder,
            "文档 two words",
            placeholder
        ]
    );
    select(&mut test, 4);
    ui::key(&test.window, Key::Home);
    test.draw();
    selected(&test, 1);
    editor_value(&test, 1, " \"literal 文档\" & ");
    ui::key(&test.window, Key::End);
    test.draw();
    selected(&test, 4);
    editor_value(&test, 4, placeholder);
    ui::popup_item(&test.window, &label(4), AccessibleRole::TextInput)
        .accessible_action(&AccessibilityAction::SetValue("last 文档 argument".into()));
    test.draw();
    assert_eq!(
        values(&test, command),
        [
            " \"literal 文档\" & ",
            placeholder,
            "文档 two words",
            "last 文档 argument"
        ]
    );
    assert!(sent.borrow().iter().all(|command| !matches!(
        command,
        Command::SavePreferences { .. } | Command::ExecuteUserCommand(_) | Command::EditFile { .. }
    )));
    render::snapshot(
        &test,
        if command {
            "command-argument-list"
        } else {
            "editor-argument-list"
        },
    );
}

#[test]
fn editor_argument_list_preserves_empty_unicode_and_reordered_selection() {
    exercise(false);
}

#[test]
fn command_argument_list_preserves_literal_values_and_placeholder_target() {
    exercise(true);
}
