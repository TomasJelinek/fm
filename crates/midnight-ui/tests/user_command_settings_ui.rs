//! Options edits use native widget actions and persist configuration only, never execute.
#[path = "support/bitmap.rs"]
mod bitmap;
#[path = "user_command_settings_ui/drag_cases.rs"]
mod drag_cases;
#[path = "support/ui.rs"]
mod ui;
use i_slint_core::{accessibility::AccessibilityAction, items::AccessibleRole};
use midnight_core::*;
use midnight_ui::{Presenter, UserCommandSettingsView, install, refresh};
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
    h.action("Apply Custom Command");
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
    h.action("Apply Custom Command");
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
    h.action("Apply Custom Command");
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
    h.action("Preview Custom Command");
    slint::platform::update_timers_and_animations();
    h.ui.draw();
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
    h.action("Apply Custom Command");
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
        "Choosing valid icon artwork must enable Update without an extra edit"
    );
    h.action("Apply Custom Command");
    assert_eq!(view.get_draft().icon, image.to_string_lossy().as_ref());
    h.action("Cancel");
    assert!(
        h.commands
            .borrow()
            .iter()
            .all(|c| !matches!(c, Command::ExecuteUserCommand(_)))
    );
}
