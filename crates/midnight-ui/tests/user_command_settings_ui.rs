//! Options edits use native widget actions and persist configuration only, never execute.
#[path = "support/bitmap.rs"]
mod bitmap;
#[path = "user_command_settings_ui/drag_cases.rs"]
mod drag_cases;
#[path = "support/popup_scroll.rs"]
mod popup_scroll;
#[path = "support/ui.rs"]
mod ui;
use i_slint_core::{accessibility::AccessibilityAction, items::AccessibleRole};
use midnight_core::*;
use midnight_ui::{Presenter, Theme, UserCommandSettingsView, install, refresh};
use slint::{ComponentHandle, Model};
use std::{cell::RefCell, rc::Rc, sync::Arc, time::Duration};

struct Harness {
    ui: ui::TestUi,
    state: Rc<RefCell<Presenter>>,
    commands: Rc<RefCell<Vec<Command>>>,
}
impl Harness {
    fn new() -> Self {
        Self::with_count(2)
    }
    fn with_count(count: usize) -> Self {
        let ui = ui::TestUi::new();
        let root = std::env::temp_dir().join("command settings UI 文档");
        let state = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
        let commands = Rc::new(RefCell::new(Vec::new()));
        let sent = commands.clone();
        install(
            &ui.window,
            state.clone(),
            Rc::new(move |command| {
                sent.borrow_mut().push(command);
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
        state.borrow_mut().apply(Event::ListingFinished {
            pane: PaneId::Left,
            navigation,
            entries: Arc::from([]),
            index: Arc::default(),
        });
        let command = |id: &str, name: &str| UserCommand {
            id: UserCommandId::new(id).unwrap(),
            name: String::from(name),
            enabled: true,
            toolbar_label: None,
            icon: None,
            working_directory: "${active.path}".into(),
            launch: UserCommandLaunch::Single,
            invocation: UserCommandInvocation::Program {
                executable: root.join("program.exe"),
                arguments: vec![],
            },
        };
        let mut configuration = vec![
            command("one", "First command"),
            command("two", "Second command"),
        ];
        for index in 2..count {
            configuration.push(command(
                &format!("extra-{index}"),
                &format!("Command {index}"),
            ));
        }
        state
            .borrow_mut()
            .apply(Event::PreferencesLoaded(Ok(Box::new(UserPreferences {
                user_commands: Arc::new(configuration),
                ..Default::default()
            }))));
        refresh(&ui.window, &state.borrow());
        commands.borrow_mut().clear();
        let mut harness = Self {
            ui,
            state,
            commands,
        };
        harness.ui.draw();
        harness
    }
    fn open(&mut self) {
        self.ui.window.invoke_show_options();
        self.ui.window.set_options_topic_index(9);
        slint::platform::update_timers_and_animations();
        self.ui.draw();
    }
    fn action(&self, label: &str) {
        ui::popup_item(&self.ui.window, label, AccessibleRole::Button)
            .accessible_action(&AccessibilityAction::Default);
        slint::platform::update_timers_and_animations();
    }
    fn text(&self, label: &str, value: &str) {
        ui::popup_item(&self.ui.window, label, AccessibleRole::TextInput)
            .accessible_action(&AccessibilityAction::SetValue(value.into()));
        slint::platform::update_timers_and_animations();
    }
    fn settle(&mut self) {
        slint::platform::update_timers_and_animations();
        std::thread::sleep(Duration::from_millis(140));
        slint::platform::update_timers_and_animations();
        self.ui.draw();
    }
    fn latest(&self) -> UserPreferences {
        self.commands
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::SavePreferences { preferences, .. } => Some(preferences.as_ref().clone()),
                _ => None,
            })
            .unwrap()
    }
    fn rows(&self) -> Vec<midnight_ui::UserCommandSettingsRow> {
        self.ui
            .window
            .global::<UserCommandSettingsView>()
            .get_rows()
            .iter()
            .collect()
    }
}

#[test]
fn new_command_is_a_selected_row_and_save_stages_it_exactly_once_until_options_ok() {
    let mut h = Harness::new();
    h.open();
    h.action("New Custom Command");
    h.ui.draw();
    bitmap::snapshot(
        &h.ui.pixels,
        h.ui.width,
        h.ui.height,
        "command-settings-pristine-new-row",
    );
    let window = h.ui.window.clone_strong();
    let view = window.global::<UserCommandSettingsView>();
    let id = view.get_draft().id;
    assert!(!id.is_empty());
    assert_eq!(h.rows().len(), 3, "New must immediately appear in the list");
    let row = h.rows().into_iter().find(|row| row.id == id).unwrap();
    assert!(row.selected);
    assert_eq!(row.name, "New command");
    assert_eq!(view.get_selected_index(), 2);

    h.text("Command name", "New command 文档");
    h.text(
        "Program executable (absolute path)",
        std::env::temp_dir()
            .join("new-program.exe")
            .to_str()
            .unwrap(),
    );
    h.settle();
    let row = h.rows().into_iter().find(|row| row.id == id).unwrap();
    assert!(row.selected);
    assert_eq!(row.name, "New command 文档");
    assert!(view.get_can_apply());
    h.action("Save Custom Command");
    h.ui.draw();
    h.action("Save Custom Command");
    assert_eq!(h.rows().len(), 3);
    assert_eq!(h.rows().iter().filter(|row| row.id == id).count(), 1);
    assert_eq!(view.get_draft().id, id);
    assert!(!view.get_dirty());
    assert_eq!(h.save_count(), 0, "Save stages the command inside Options");

    h.action("OK");
    assert_eq!(h.save_count(), 1);
    let saved = h.latest();
    assert_eq!(saved.user_commands.len(), 3);
    assert_eq!(
        saved
            .user_commands
            .iter()
            .filter(|command| command.id.as_str() == id.as_str())
            .count(),
        1
    );
    assert_eq!(saved.user_commands[2].name, "New command 文档");
    assert!(h.commands.borrow().iter().all(|command| !matches!(
        command,
        Command::ExecuteUserCommand(_) | Command::PreviewUserCommand(_)
    )));
}

#[test]
fn discard_removes_a_new_row_and_options_cancel_removes_a_staged_new_command() {
    let mut h = Harness::new();
    h.open();
    h.action("New Custom Command");
    h.ui.draw();
    let discarded =
        h.ui.window
            .global::<UserCommandSettingsView>()
            .get_draft()
            .id;
    h.text("Command name", "Discard this new command");
    h.settle();
    h.action("Cancel Custom Command draft");
    h.ui.draw();
    assert_eq!(h.rows().len(), 2);
    assert!(h.rows().iter().all(|row| row.id != discarded));

    h.action("New Custom Command");
    h.ui.draw();
    let staged =
        h.ui.window
            .global::<UserCommandSettingsView>()
            .get_draft()
            .id;
    h.text("Command name", "Cancel this staged command");
    h.text(
        "Program executable (absolute path)",
        std::env::temp_dir()
            .join("cancelled-program.exe")
            .to_str()
            .unwrap(),
    );
    h.settle();
    h.action("Save Custom Command");
    h.ui.draw();
    assert_eq!(h.rows().iter().filter(|row| row.id == staged).count(), 1);
    h.action("Cancel");
    h.open();
    assert_eq!(h.rows().len(), 2);
    assert!(h.rows().iter().all(|row| row.id != staged));
    assert_eq!(h.rows()[0].id, "one");
    assert_eq!(h.rows()[0].name, "First command");
    assert_eq!(h.save_count(), 0);
    assert!(h.commands.borrow().iter().all(|command| !matches!(
        command,
        Command::ExecuteUserCommand(_) | Command::PreviewUserCommand(_)
    )));
}

#[test]
fn required_value_labels_follow_the_selected_shell_and_toggle_the_declared_capture() {
    let mut h = Harness::new();
    h.open();
    let window = h.ui.window.clone_strong();
    let view = window.global::<UserCommandSettingsView>();
    let mut form = view.get_draft();
    form.kind = 1;
    form.script = "echo captured values".into();
    view.set_draft(form.clone());
    view.invoke_draft_edited(view.get_token(), form);
    h.ui.draw();
    h.action("Required terminal values");
    let labels = [
        "Active folder",
        "Other folder",
        "File path",
        "File name",
        "Name without extension",
        "Extension",
    ];
    let references = [
        [
            "\"$env:ME_ACTIVE_PATH\"",
            "\"$env:ME_OTHER_PATH\"",
            "\"$env:ME_FILE_PATH\"",
            "\"$env:ME_FILE_NAME\"",
            "\"$env:ME_FILE_STEM\"",
            "\"$env:ME_FILE_EXTENSION\"",
        ],
        [
            "\"$ME_ACTIVE_PATH\"",
            "\"$ME_OTHER_PATH\"",
            "\"$ME_FILE_PATH\"",
            "\"$ME_FILE_NAME\"",
            "\"$ME_FILE_STEM\"",
            "\"$ME_FILE_EXTENSION\"",
        ],
        [
            "!ME_ACTIVE_PATH_ARG!",
            "!ME_OTHER_PATH_ARG!",
            "!ME_FILE_PATH_ARG!",
            "!ME_FILE_NAME_ARG!",
            "!ME_FILE_STEM_ARG!",
            "!ME_FILE_EXTENSION_ARG!",
        ],
    ];
    for (shell, references) in references.into_iter().enumerate() {
        let mut form = view.get_draft();
        form.shell = shell as i32;
        view.set_draft(form.clone());
        view.invoke_draft_edited(view.get_token(), form);
        h.ui.draw();
        for (index, (label, reference)) in labels.into_iter().zip(references).enumerate() {
            let checkbox = ui::popup_item(
                &h.ui.window,
                &format!("{label} · {reference}"),
                AccessibleRole::Checkbox,
            );
            checkbox.accessible_action(&AccessibilityAction::Default);
            h.ui.draw();
            let form = view.get_draft();
            let captures = [
                form.active_path,
                form.other_path,
                form.file_path,
                form.file_name,
                form.file_stem,
                form.file_extension,
            ];
            assert_eq!(captures.iter().filter(|checked| **checked).count(), 1);
            assert!(
                captures[index],
                "{label} must require its own captured value"
            );
            checkbox.accessible_action(&AccessibilityAction::Default);
            h.ui.draw();
        }
    }
    assert!(h.commands.borrow().iter().all(|command| !matches!(
        command,
        Command::ExecuteUserCommand(_)
            | Command::PreviewUserCommand(_)
            | Command::SavePreferences { .. }
    )));
}

#[test]
fn working_folder_variables_stay_beside_browse_and_replace_the_field_in_a_small_window() {
    let mut h = Harness::new();
    h.open();
    for (theme, density, font) in [(0, 0, 12), (1, 1, 15), (3, 0, 32), (3, 1, 32)] {
        h.ui.window.invoke_apply_preferences(theme, density);
        h.ui.window.invoke_apply_fonts("".into(), font, font);
        h.ui.resize(320, 200);
        for (label, expected) in [
            ("Active folder", "${active.path}"),
            ("Other folder", "${other.path}"),
        ] {
            h.text("Working folder", "replace this whole field");
            h.settle();
            let browse = ui::popup_item(
                &h.ui.window,
                "Browse working directory",
                AccessibleRole::Button,
            );
            let variables = ui::popup_item(
                &h.ui.window,
                "Working directory variables",
                AccessibleRole::Combobox,
            );
            popup_scroll::click(
                &mut h.ui,
                "Working directory variables",
                AccessibleRole::Combobox,
            );
            bitmap::snapshot(
                &h.ui.pixels,
                h.ui.width,
                h.ui.height,
                &format!(
                    "command-settings-folder-variables-theme{theme}-density{density}-font{font}-{}",
                    if label == "Active folder" {
                        "active"
                    } else {
                        "other"
                    }
                ),
            );
            let browse_rect = browse.geometry();
            let browse_at = browse.map_to_native_window(browse_rect.origin);
            let variables_rect = variables.geometry();
            let variables_at = variables.map_to_native_window(variables_rect.origin);
            assert!((browse_at.y - variables_at.y).abs() <= 1.0);
            assert!((browse_rect.height() - variables_rect.height()).abs() <= 1.0);
            assert!(browse_at.x + browse_rect.width() <= variables_at.x + 1.0);
            assert!(variables_at.x - browse_at.x - browse_rect.width() <= 8.0);
            assert!(
                variables_rect.width()
                    <= h.ui.window.global::<Theme>().get_control_height().max(32.0) + 1.0
            );
            assert!(browse_at.x >= 0.0 && browse_at.y >= 0.0);
            assert!(
                variables_at.x + variables_rect.width() <= 320.0
                    && variables_at.y + variables_rect.height() <= 200.0
            );
            popup_scroll::click(&mut h.ui, label, AccessibleRole::Button);
            assert_eq!(
                h.ui.window
                    .global::<UserCommandSettingsView>()
                    .get_draft()
                    .working_directory,
                expected
            );
            assert!(h.ui.window.get_options_open());
            assert!(!h.ui.window.get_operations_visible());
        }
    }
    assert!(h.commands.borrow().iter().all(|command| !matches!(
        command,
        Command::ExecuteUserCommand(_)
            | Command::PreviewUserCommand(_)
            | Command::SavePreferences { .. }
    )));
}

#[test]
fn save_is_a_visible_primary_action_and_preview_keeps_the_secondary_palette() {
    let mut h = Harness::new();
    h.open();
    h.text("Command name", "Stage through the visible Save");
    h.settle();
    popup_scroll::click(&mut h.ui, "Save Custom Command", AccessibleRole::Button);
    h.text("Command name", "Keep Save enabled for the palette check");
    h.settle();
    let theme = h.ui.window.global::<Theme>();
    for (label, expected) in [
        ("Save Custom Command", theme.get_active_header()),
        ("Preview Custom Command", theme.get_secondary_surface()),
    ] {
        let button = ui::popup_item(&h.ui.window, label, AccessibleRole::Button);
        let origin = button.map_to_native_window(button.geometry().origin);
        let pixel = h.ui.pixel(origin.x + 5.0, origin.y + 5.0);
        assert_eq!(
            (pixel.r, pixel.g, pixel.b),
            (expected.red(), expected.green(), expected.blue()),
            "{label} must use its application action palette"
        );
        if label == "Save Custom Command" {
            assert!(
                ui::visible_text_values(button)
                    .iter()
                    .any(|text| text == "Save")
            );
        }
    }
    assert_eq!(h.save_count(), 0);
    assert!(h.commands.borrow().iter().all(|command| !matches!(
        command,
        Command::ExecuteUserCommand(_) | Command::PreviewUserCommand(_)
    )));
}

#[test]
fn cancel_restores_applied_command_edits_and_order() {
    let mut h = Harness::new();
    h.open();
    let window = h.ui.window.clone_strong();
    let view = window.global::<UserCommandSettingsView>();
    h.action("Move Custom Command down");
    // Settle the new draft revision before the next user edit starts validation.
    h.ui.draw();
    assert_eq!(h.rows()[0].id, "two");
    h.text("Command name", "Previewed command name");
    h.settle();
    assert!(
        view.get_can_apply(),
        "draft name={:?}, dirty={}, error={:?}, validation={:?}",
        view.get_draft().name,
        view.get_dirty(),
        view.get_error(),
        view.get_validation()
    );
    h.action("Save Custom Command");
    assert!(
        h.rows()
            .iter()
            .any(|row| row.name == "Previewed command name")
    );
    assert_eq!(h.save_count(), 0);
    h.action("Cancel");
    h.open();
    assert_eq!(h.rows()[0].id, "one");
    assert_eq!(h.rows()[0].name, "First command");
    assert_eq!(h.save_count(), 0);
}

#[test]
fn batched_accessibility_edit_after_reorder_keeps_current_draft_validation() {
    let mut h = Harness::new();
    h.open();
    let window = h.ui.window.clone_strong();
    let view = window.global::<UserCommandSettingsView>();
    // AccessKit can deliver consecutive actions before the next event-loop tick.
    // Leave the reorder's draft-revision notification pending while editing.
    view.invoke_move(view.get_token(), 1);
    h.text("Command name", "Batched accessible command");
    h.settle();
    assert!(
        view.get_can_apply(),
        "current draft validation was cancelled: name={:?}, dirty={}, error={:?}, validation={:?}",
        view.get_draft().name,
        view.get_dirty(),
        view.get_error(),
        view.get_validation()
    );
    h.action("Save Custom Command");
    assert_eq!(h.rows()[1].id, "one");
    assert_eq!(h.rows()[1].name, "Batched accessible command");
    assert_eq!(h.save_count(), 0);
}

#[test]
fn options_create_edit_reorder_delete_use_stable_ids_and_preserve_text_draft_on_validation() {
    let mut h = Harness::new();
    h.ui.window.invoke_show_options();
    h.ui.draw();
    ui::click_popup(&h.ui.window, "Custom Commands", AccessibleRole::Button);
    h.ui.draw();
    assert_eq!(h.ui.window.get_options_topic_index(), 9);
    h.action("New Custom Command");
    h.ui.draw();
    h.text("Command name", "New command 文档");
    let executable = std::env::temp_dir().join("new-program.exe");
    h.text(
        "Program executable (absolute path)",
        executable.to_str().unwrap(),
    );
    h.settle();
    let view = h.ui.window.global::<UserCommandSettingsView>();
    assert!(
        view.get_can_apply(),
        "draft name={:?}, executable={:?}, dirty={}, error={:?}, validation={:?}",
        view.get_draft().name,
        view.get_draft().executable,
        view.get_dirty(),
        view.get_error(),
        view.get_validation()
    );
    let id = view.get_draft().id;
    h.action("Save Custom Command");
    h.ui.draw();
    assert_eq!(h.rows().len(), 3);
    assert_eq!(h.rows()[2].id.as_str(), id.as_str());
    h.action("Move Custom Command up");
    h.ui.draw();
    assert_eq!(h.rows()[1].id.as_str(), id.as_str());
    h.text("Command name", "Renamed without reset");
    h.text("Program executable (absolute path)", "");
    h.settle();
    let view = h.ui.window.global::<UserCommandSettingsView>();
    assert!(!view.get_can_apply());
    assert_eq!(view.get_draft().name, "Renamed without reset");
    assert!(!view.get_validation().is_empty());
    h.action("Cancel Custom Command draft");
    h.ui.draw();
    assert_eq!(
        h.ui.window
            .global::<UserCommandSettingsView>()
            .get_draft()
            .name,
        "New command 文档"
    );
    h.action("Delete Custom Command");
    h.ui.draw();
    assert_eq!(h.rows().len(), 2);
    assert_eq!(
        h.save_count(),
        0,
        "all page actions remain in the edit session"
    );
    h.action("OK");
    assert_eq!(h.latest().user_commands.len(), 2);
    assert!(
        h.commands
            .borrow()
            .iter()
            .all(|command| !matches!(command, Command::ExecuteUserCommand(_)))
    );
}

#[test]
fn terminal_variable_preview_uses_unsaved_literal_script_and_does_not_save_or_execute() {
    let mut h = Harness::new();
    h.open();
    let view = h.ui.window.global::<UserCommandSettingsView>();
    let mut form = view.get_draft();
    form.kind = 1;
    form.shell = 0;
    form.script = "Write-Output".into();
    view.set_draft(form);
    let token = view.get_token();
    assert!(view.invoke_insert_variable(token.clone(), view.get_draft(), 2, 0));
    assert_eq!(
        view.get_draft().script,
        "Write-Output \"$env:ME_ACTIVE_PATH\""
    );
    assert!(view.get_draft().active_path);
    view.invoke_validate(token, view.get_draft());
    assert!(view.get_can_preview());
    assert!(
        h.commands
            .borrow()
            .iter()
            .all(|command| !matches!(command, Command::PreviewUserCommand(_)))
    );
    h.action("Preview Custom Command");
    slint::platform::update_timers_and_animations();
    h.ui.draw();
    assert!(h.ui.window.get_options_open());
    assert!(!h.ui.window.get_operations_visible());
    let commands = h.commands.borrow();
    let request = commands
        .iter()
        .find_map(|command| match command {
            Command::PreviewUserCommand(request) => Some(request),
            _ => None,
        })
        .unwrap();
    let UserCommandInvocation::Terminal {
        script, variables, ..
    } = &request.configuration[0].invocation
    else {
        panic!()
    };
    assert_eq!(script, "Write-Output \"$env:ME_ACTIVE_PATH\"");
    assert_eq!(variables, &[UserCommandVariable::ActivePath]);
    assert!(commands.iter().all(|command| !matches!(
        command,
        Command::ExecuteUserCommand(_) | Command::SavePreferences { .. }
    )));
}

#[test]
fn editor_page_and_done_remain_accessible_in_narrow_layout_and_every_theme_density() {
    let mut h = Harness::new();
    h.open();
    for theme in midnight_core::PreferenceTheme::ALL.map(|theme| theme.index()) {
        for density in 0..2 {
            h.ui.window.invoke_apply_preferences(theme, density);
            for (width, height) in [(1340, 820), (720, 500), (320, 200)] {
                h.ui.resize(width, height);
                h.ui.draw();
                bitmap::snapshot(
                    &h.ui.pixels,
                    width,
                    height,
                    &format!("command-settings-theme{theme}-density{density}-{width}x{height}"),
                );
                assert_eq!(h.ui.window.get_options_topic_index(), 9);
                let view = h.ui.window.global::<UserCommandSettingsView>();
                assert_eq!(view.get_rows().row_count(), 2);
                let done = ui::popup_item(&h.ui.window, "Cancel", AccessibleRole::Button);
                let rect = done.geometry();
                let origin = done.map_to_native_window(rect.origin);
                assert!(
                    origin.x >= 0.0
                        && origin.y >= 0.0
                        && origin.x + rect.size.width <= width as f32 + 1.0
                        && origin.y + rect.size.height <= height as f32 + 1.0
                );
                assert!(view.get_has_draft());
            }
        }
    }
    h.action("Cancel");
}

#[test]
fn dragging_a_command_after_another_row_commits_one_ordered_preferences_transaction() {
    use slint::platform::{PointerEventButton, WindowEvent};
    let mut h = Harness::new();
    h.open();
    let first = ui::popup_item(
        &h.ui.window,
        "First command · Program · program.exe · Single",
        AccessibleRole::ListItem,
    );
    let second = ui::popup_item(
        &h.ui.window,
        "Second command · Program · program.exe · Single",
        AccessibleRole::ListItem,
    );
    let a = first.geometry();
    let a = first.map_to_native_window(a.origin);
    let b = second.geometry();
    let b_height = b.size.height;
    let b = second.map_to_native_window(b.origin);
    let start = slint::LogicalPosition::new(a.x + 36.0, a.y + 12.0);
    let end = slint::LogicalPosition::new(b.x + 36.0, b.y + b_height - 2.0);
    h.ui.window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position: start });
    h.ui.window
        .window()
        .dispatch_event(WindowEvent::PointerPressed {
            position: start,
            button: PointerEventButton::Left,
        });
    h.ui.window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position: end });
    h.ui.window
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position: end,
            button: PointerEventButton::Left,
        });
    slint::platform::update_timers_and_animations();
    h.ui.draw();
    let count = h
        .commands
        .borrow()
        .iter()
        .filter(|command| matches!(command, Command::SavePreferences { .. }))
        .count();
    assert_eq!(count, 0, "dragging stages the order until OK");
    assert_eq!(
        h.rows()
            .iter()
            .map(|command| command.id.as_str())
            .collect::<Vec<_>>(),
        ["two", "one"]
    );
}

#[test]
fn ancestor_clipped_horizontal_drop_cancels_outside_the_actual_window() {
    use slint::platform::{PointerEventButton, WindowEvent};
    let mut h = Harness::new();
    h.open();
    h.ui.resize(320, 500);
    slint::platform::update_timers_and_animations();
    h.ui.draw();
    let first = ui::popup_item(
        &h.ui.window,
        "First command · Program · program.exe · Single",
        AccessibleRole::ListItem,
    );
    let second = ui::popup_item(
        &h.ui.window,
        "Second command · Program · program.exe · Single",
        AccessibleRole::ListItem,
    );
    let a = first.geometry();
    let a = first.map_to_native_window(a.origin);
    let b = second.geometry();
    let height = b.size.height;
    let b = second.map_to_native_window(b.origin);
    let start = slint::LogicalPosition::new(a.x + 20.0, a.y + 8.0);
    let end = slint::LogicalPosition::new(340.0, b.y + height - 2.0);
    assert!(start.x >= 0.0 && start.x < 320.0 && start.y >= 0.0 && start.y < 500.0);
    h.ui.window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position: start });
    h.ui.window
        .window()
        .dispatch_event(WindowEvent::PointerPressed {
            position: start,
            button: PointerEventButton::Left,
        });
    assert!(
        h.ui.window
            .global::<UserCommandSettingsView>()
            .get_drag_active(),
        "press starts on a real visible row"
    );
    h.ui.window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position: end });
    h.ui.window
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position: end,
            button: PointerEventButton::Left,
        });
    slint::platform::update_timers_and_animations();
    h.ui.draw();
    assert_eq!(
        h.commands
            .borrow()
            .iter()
            .filter(|c| matches!(c, Command::SavePreferences { .. }))
            .count(),
        0,
        "a drop outside the actual window must not reorder ancestor-clipped content"
    );
}

#[test]
fn command_priority_list_is_single_line_with_compact_actions_above_it() {
    let mut h = Harness::with_count(8);
    h.open();
    let row = ui::popup_item(
        &h.ui.window,
        "First command · Program · program.exe · Single",
        AccessibleRole::ListItem,
    );
    let input = ui::popup_item(&h.ui.window, "Command name", AccessibleRole::TextInput);
    assert!(
        row.geometry().height() <= input.geometry().height() + 2.0,
        "Command priority rows must fit one control line, without a second summary line"
    );
    let list = ui::popup_item(
        &h.ui.window,
        "Configured Custom Commands",
        AccessibleRole::List,
    );
    let list_top = list.map_to_native_window(list.geometry().origin).y;
    for label in [
        "New Custom Command",
        "Delete Custom Command",
        "Move Custom Command up",
        "Move Custom Command down",
    ] {
        let button = ui::popup_item(&h.ui.window, label, AccessibleRole::Button);
        let origin = button.map_to_native_window(button.geometry().origin);
        assert!(
            origin.y + button.geometry().height() <= list_top + 1.0,
            "Priority actions belong in the list header: {label}"
        );
        assert!(
            button.geometry().width() <= 48.0,
            "Priority actions must be compact"
        );
    }
    bitmap::snapshot(
        &h.ui.pixels,
        h.ui.width,
        h.ui.height,
        "command-settings-compact-priority",
    );
    assert!(
        h.commands
            .borrow()
            .iter()
            .all(|c| !matches!(c, Command::ExecuteUserCommand(_)))
    );
}

#[test]
fn separator_details_keep_mode_drafts_and_never_preview_or_execute() {
    let mut h = Harness::new();
    h.open();
    let window = h.ui.window.clone_strong();
    let view = window.global::<UserCommandSettingsView>();
    let mut form = view.get_draft();
    let original_executable = form.executable.clone();
    form.arguments = r#"["two words", "", "a \"quote\""]"#.into();
    form.script = "echo preserved".into();
    form.kind = 1;
    view.set_draft(form.clone());
    view.invoke_draft_edited(view.get_token(), form.clone());
    h.ui.draw();
    assert!(
        ui::popup_item(
            &h.ui.window,
            "Custom Command script",
            AccessibleRole::TextInput
        )
        .is_visible()
    );
    form.kind = 0;
    view.set_draft(form.clone());
    view.invoke_draft_edited(view.get_token(), form.clone());
    h.ui.draw();
    assert_eq!(view.get_draft().executable, original_executable);
    assert_eq!(view.get_draft().script, "echo preserved");
    assert_eq!(view.get_argument_rows().row_count(), 3);
    form.kind = 2;
    view.set_draft(form.clone());
    view.invoke_draft_edited(view.get_token(), form.clone());
    view.invoke_validate(view.get_token(), form.clone());
    h.ui.draw();
    assert!(view.get_can_apply());
    assert!(!view.get_can_preview());
    assert!(!view.invoke_preview(view.get_token(), form.clone()));
    assert!(h.commands.borrow().iter().all(|c| !matches!(
        c,
        Command::ExecuteUserCommand(_) | Command::PreviewUserCommand(_)
    )));
    h.action("Save Custom Command");
    h.ui.draw();
    assert!(h.rows()[0].separator);
    bitmap::snapshot(
        &h.ui.pixels,
        h.ui.width,
        h.ui.height,
        "command-settings-separator",
    );
    h.action("Cancel");
    h.open();
    assert!(
        !h.rows()[0].separator,
        "Options Cancel restores the original command"
    );
}

#[test]
fn icon_artwork_uses_managed_picker_and_valid_completion_enables_update() {
    let mut h = Harness::new();
    h.open();
    h.action("Toolbar appearance");
    h.ui.draw();
    assert!(
        ui::try_popup_item(
            &h.ui.window,
            "Icon image (optional absolute path)",
            AccessibleRole::TextInput
        )
        .is_none(),
        "Icon imports must not be bypassed by editing a path"
    );
    h.action("Browse command icon");
    let request = h
        .commands
        .borrow()
        .iter()
        .rev()
        .find_map(|command| match command {
            Command::PickPath { request, input } => {
                assert_eq!(input.kind, PathPickerKind::CommandIconImage);
                Some(*request)
            }
            _ => None,
        })
        .unwrap();
    let image = std::env::temp_dir().join("managed-command-artwork.png");
    h.state.borrow_mut().apply(Event::PathPicked {
        request,
        result: Ok(Some(image.clone())),
    });
    refresh(&h.ui.window, &h.state.borrow());
    h.ui.draw();
    let window = h.ui.window.clone_strong();
    let view = window.global::<UserCommandSettingsView>();
    assert!(
        view.get_can_apply(),
        "Choosing valid icon artwork must enable Save without an extra edit"
    );
    h.action("Save Custom Command");
    assert_eq!(view.get_draft().icon, image.to_string_lossy().as_ref());
    h.action("Cancel");
    assert!(
        h.commands
            .borrow()
            .iter()
            .all(|c| !matches!(c, Command::ExecuteUserCommand(_)))
    );
}
