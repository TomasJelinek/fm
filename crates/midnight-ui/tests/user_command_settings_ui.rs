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
    assert_eq!(h.latest().user_commands.len(), 3);
    assert_eq!(h.latest().user_commands[2].id.as_str(), id.as_str());
    h.action("Move Custom Command up");
    h.ui.draw();
    assert_eq!(h.latest().user_commands[1].id.as_str(), id.as_str());
    h.text("Command name", "Renamed without reset");
    h.text("Custom Command arguments", "[");
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
    for theme in 0..9 {
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
                let done = ui::popup_item(&h.ui.window, "Done", AccessibleRole::Button);
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
    h.action("Done");
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
    assert_eq!(count, 1, "one valid drag drop must save exactly once");
    assert_eq!(
        h.latest()
            .user_commands
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
