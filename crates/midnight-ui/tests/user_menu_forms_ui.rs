#[path = "support/bitmap.rs"]
mod bitmap;
#[path = "support/popup_scroll.rs"]
mod popup_scroll;
#[path = "support/ui.rs"]
mod ui;
use i_slint_core::{
    accessibility::{AccessibilityAction, AccessibleStringProperty},
    item_tree::ItemRc,
    items::AccessibleRole,
    window::{WindowInner, WindowKind},
};
use midnight_core::*;
use midnight_ui::{Presenter, UserCommandSettingsView, install, refresh};
use slint::{ComponentHandle, Model};
use std::{cell::RefCell, rc::Rc, sync::Arc};

fn options() -> (ui::TestUi, Rc<RefCell<Presenter>>) {
    options_with_editor_arguments(vec!["--literal flag".into(), "{file}".into()])
}
fn options_with_editor_arguments(arguments: Vec<String>) -> (ui::TestUi, Rc<RefCell<Presenter>>) {
    options_with_sender(arguments, Rc::new(|_| true))
}
fn options_with_sender(
    arguments: Vec<String>,
    send: Rc<dyn Fn(Command) -> bool>,
) -> (ui::TestUi, Rc<RefCell<Presenter>>) {
    let mut ui = ui::TestUi::new();
    let root = std::env::current_dir().unwrap();
    let p = Rc::new(RefCell::new(Presenter::new(root.clone(), root)));
    install(&ui.window, p.clone(), send);
    let preferences = UserPreferences {
        editor: EditorPreferences {
            rules: Vec::new(),
            executable: Some(std::env::current_exe().unwrap()),
            arguments,
        },
        user_commands: Arc::new(vec![UserCommand {
            id: UserCommandId::new("menu-example").unwrap(),
            name: "Menu example".into(),
            enabled: false,
            toolbar_label: None,
            icon: None,
            working_directory: "${active.path}".into(),
            launch: UserCommandLaunch::Single,
            invocation: UserCommandInvocation::Program {
                executable: std::env::current_exe().unwrap(),
                arguments: vec!["--literal flag".into(), "${file.path}".into()],
            },
        }]),
        ..Default::default()
    };
    p.borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::new(preferences))));
    refresh(&ui.window, &p.borrow());
    ui.window.invoke_show_options();
    ui.draw();
    (ui, p)
}
fn no_json(ui: &ui::TestUi) {
    let root = {
        let inner = WindowInner::from_pub(ui.window.window());
        inner
            .active_popups()
            .iter()
            .rev()
            .find(|p| !matches!(p.window_kind, WindowKind::ToolTip))
            .unwrap()
            .component
            .clone()
    };
    let labels = ui::visible_text_values(ItemRc::new_root(root));
    assert!(
        !labels.iter().any(|text| text.contains("JSON")),
        "ordinary settings must show argument rows, not JSON: {labels:?}"
    );
}

fn reveal_option(h: &mut ui::TestUi, label: &str, role: AccessibleRole) {
    use i_slint_core::{item_tree::ParentItemTraversalMode, items::Flickable};
    use slint::platform::WindowEvent;
    for _ in 0..8 {
        let row = ui::popup_item(&h.window, label, role);
        let mut parent = row.parent_item(ParentItemTraversalMode::StopAtPopups);
        let mut outer = None;
        while let Some(item) = parent {
            parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
            if item.downcast::<Flickable>().is_some() {
                outer = Some(item);
            }
        }
        let outer = outer.expect("scrollable Options page");
        let viewport = outer.geometry();
        let origin = outer.map_to_native_window(viewport.origin);
        let rect = row.geometry();
        let point = row.map_to_native_window(rect.origin);
        if point.y >= origin.y && point.y + rect.size.height <= origin.y + viewport.size.height {
            return;
        }
        h.window
            .window()
            .dispatch_event(WindowEvent::PointerScrolled {
                position: slint::LogicalPosition::new(
                    origin.x + viewport.size.width - 24.0,
                    origin.y + 4.0,
                ),
                delta_x: 0.0,
                delta_y: if point.y < origin.y { 1000.0 } else { -1000.0 },
            });
        h.draw();
    }
    panic!("could not reveal {label}");
}

#[test]
fn command_preview_stays_inside_options_and_editing_clears_stale_results() {
    let commands = Rc::new(RefCell::new(Vec::new()));
    let sent = commands.clone();
    let (mut h, p) = options_with_sender(
        vec!["{file}".into()],
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
        .expect("initial panel listing request");
    p.borrow_mut().apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation,
        entries: Arc::from([]),
        index: Arc::default(),
    });
    refresh(&h.window, &p.borrow());
    h.window.set_options_topic_index(9);
    h.draw();
    reveal_option(&mut h, "Preview Custom Command", AccessibleRole::Button);
    assert!(
        h.window
            .global::<UserCommandSettingsView>()
            .get_can_preview(),
        "preview disabled: {} / {}",
        h.window.global::<UserCommandSettingsView>().get_error(),
        h.window
            .global::<UserCommandSettingsView>()
            .get_validation()
    );
    bitmap::snapshot(
        &h.pixels,
        h.width,
        h.height,
        "custom-command-before-preview",
    );
    ui::click_popup(&h.window, "Preview Custom Command", AccessibleRole::Button);
    h.draw();
    let (request, command) = commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::PreviewUserCommand(request) => Some((request.id, request.command.clone())),
            _ => None,
        })
        .expect("preview must reach backend without launching a process");
    p.borrow_mut()
        .apply(Event::UserCommand(Box::new(UserCommandEvent {
            request,
            command,
            update: UserCommandUpdate::Preview(Arc::new(UserCommandPreview {
                processes: vec![Arc::new(UserCommandPreviewRow {
                    process: UserCommandProcess {
                        working_directory: std::env::current_dir().unwrap(),
                        invocation: UserCommandProcessInvocation::Program {
                            executable: std::env::current_exe().unwrap(),
                            arguments: vec!["separate 文档 argument".into()],
                        },
                    },
                    retention: UserCommandRetention::new(()),
                })]
                .into(),
                retention: UserCommandRetention::new(()),
            })),
        })));
    refresh(&h.window, &p.borrow());
    for _ in 0..3 {
        h.draw();
    }
    assert!(h.window.get_options_open());
    assert!(!h.window.get_operations_visible());
    assert!(
        h.window
            .global::<UserCommandSettingsView>()
            .get_preview_text()
            .contains("separate 文档 argument")
    );
    assert!(
        h.window
            .global::<UserCommandSettingsView>()
            .get_preview_text()
            .contains("no process was started")
    );
    let result = ui::popup_item(
        &h.window,
        "Custom Command preview result",
        AccessibleRole::Text,
    );
    let rect = result.geometry();
    let point = result.map_to_native_window(rect.origin);
    assert!(
        point.y > 0.0 && point.y + rect.size.height < h.height as f32 - 40.0,
        "preview must be revealed inside the page: {point:?}/{rect:?}"
    );
    drop(result);
    bitmap::snapshot(
        &h.pixels,
        h.width,
        h.height,
        "custom-command-inline-preview",
    );
    h.resize(720, 500);
    reveal_option(
        &mut h,
        "Custom Command preview result",
        AccessibleRole::Text,
    );
    bitmap::snapshot(
        &h.pixels,
        h.width,
        h.height,
        "custom-command-inline-preview-compact",
    );
    assert!(h.window.get_options_open());
    assert!(!h.window.get_operations_visible());
    h.resize(1340, 820);
    assert!(commands.borrow().iter().all(|command| !matches!(
        command,
        Command::ExecuteUserCommand(_) | Command::SavePreferences { .. }
    )));
    reveal_option(&mut h, "Command name", AccessibleRole::TextInput);
    ui::click_popup(&h.window, "Command name", AccessibleRole::TextInput);
    ui::chord(&h.window, &[slint::platform::Key::Control], "a");
    ui::key(&h.window, "Continued editing");
    h.draw();
    assert_eq!(
        h.window
            .global::<UserCommandSettingsView>()
            .get_draft()
            .name,
        "Continued editing"
    );
    assert!(
        h.window
            .global::<UserCommandSettingsView>()
            .get_preview_text()
            .is_empty()
    );
    assert!(h.window.get_options_open());
    ui::click_popup(&h.window, "Cancel", AccessibleRole::Button);
    h.draw();
    assert!(!h.window.get_options_open());
    assert!(
        commands
            .borrow()
            .iter()
            .all(|command| !matches!(command, Command::SavePreferences { .. }))
    );
}
#[test]
fn editor_settings_use_argument_rows_and_native_browse_controls() {
    let (mut h, _p) = options();
    ui::click_popup(&h.window, "Editors", AccessibleRole::Button);
    h.draw();
    no_json(&h);
    ui::popup_item(&h.window, "Editor argument 1", AccessibleRole::TextInput);
    ui::popup_item(
        &h.window,
        "Browse editor executable",
        AccessibleRole::Button,
    );
}
#[test]
fn user_menu_has_selected_command_fields_argument_rows_and_per_field_browse() {
    let (mut h, _p) = options();
    ui::click_popup(&h.window, "Custom Commands", AccessibleRole::Button);
    h.draw();
    no_json(&h);
    ui::popup_item(&h.window, "Command name", AccessibleRole::TextInput);
    ui::popup_item(&h.window, "Command argument 1", AccessibleRole::TextInput);
    for label in ["Browse command executable", "Browse working directory"] {
        ui::popup_item(&h.window, label, AccessibleRole::Button);
    }
    assert!(ui::try_popup_item(&h.window, "Browse command icon", AccessibleRole::Button).is_none());
    reveal_option(&mut h, "Toolbar appearance", AccessibleRole::Button);
    ui::click_popup(&h.window, "Toolbar appearance", AccessibleRole::Button);
    h.draw();
    for label in [
        "Change command icon",
        "Browse command icon",
        "Clear command icon",
    ] {
        reveal_option(&mut h, label, AccessibleRole::Button);
        ui::popup_item(&h.window, label, AccessibleRole::Button);
    }
}

#[test]
fn editor_argument_rows_have_real_scroll_extent_and_last_row_can_be_reached_by_wheel() {
    use i_slint_core::{item_tree::ParentItemTraversalMode, items::Flickable};
    use slint::platform::WindowEvent;
    let mut arguments = vec![String::new(); 64];
    arguments[0] = "{file}".into();
    let (mut h, _p) = options_with_editor_arguments(arguments);
    h.window.set_options_topic_index(4);
    h.draw();
    // Begin with an instantiated list delegate; the last row may be virtualized
    // until the list itself (not the surrounding settings page) is scrolled.
    let row = ui::popup_item(&h.window, "Editor argument 1", AccessibleRole::ListItem);
    let mut parent = row.parent_item(ParentItemTraversalMode::StopAtPopups);
    let outer = loop {
        let item = parent.expect("argument rows must have a scrollable Options viewport");
        if item.downcast::<Flickable>().is_some() {
            break item;
        }
        parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    };
    drop(row);
    let geometry = outer.geometry();
    let origin = outer.map_to_native_window(geometry.origin);
    assert!(
        outer
            .downcast::<Flickable>()
            .unwrap()
            .as_pin_ref()
            .content_height()
            .get()
            > geometry.size.height * 2.0
    );
    h.window
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position: slint::LogicalPosition::new(origin.x + 20.0, origin.y + 20.0),
            delta_x: 0.0,
            delta_y: -100_000.0,
        });
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        h.draw();
    }
    let row = ui::popup_item(&h.window, "Editor argument 64", AccessibleRole::ListItem);
    let rect = row.geometry();
    let point = row.map_to_native_window(rect.origin);
    assert!(
        point.y >= origin.y && point.y + rect.size.height <= origin.y + geometry.size.height + 0.5,
        "wheel must reveal the last complete argument inside its list viewport: row={point:?}/{rect:?}, viewport={origin:?}/{geometry:?}"
    );
    row.accessible_action(&AccessibilityAction::Default);
    h.draw();
    assert_eq!(
        ui::popup_item(&h.window, "Editor argument 64", AccessibleRole::TextInput)
            .accessible_string_property(AccessibleStringProperty::Value)
            .as_deref(),
        Some(""),
        "the wheel-reached final row must become the editable empty argument"
    );
}

#[test]
fn selected_forms_keep_save_and_close_in_window_in_compact_layouts() {
    let (mut h, _p) = options();
    for topic in [4, 9] {
        h.window.set_options_topic_index(topic);
        for (width, height) in [(1340, 820), (720, 500), (320, 200)] {
            h.resize(width, height);
            h.draw();
            bitmap::snapshot(
                &h.pixels,
                width,
                height,
                &format!("user-menu-form-{topic}-{width}x{height}"),
            );
            for label in ["OK", "Cancel"] {
                let item = ui::popup_item(&h.window, label, AccessibleRole::Button);
                let rect = item.geometry();
                let position = item.map_to_native_window(rect.origin);
                assert!(
                    position.x >= 0.0
                        && position.y >= 0.0
                        && position.x + rect.size.width <= width as f32 + 1.0
                        && position.y + rect.size.height <= height as f32 + 1.0
                );
            }
        }
    }
}

#[test]
fn hovered_argument_can_be_removed_without_popup_reentrancy() {
    use slint::platform::WindowEvent;
    let (mut h, _p) = options();
    h.window.set_options_topic_index(9);
    h.draw();
    // Reveal each nested clip by its required distance, then select by real
    // pointer input before testing removal with the row tooltip still alive.
    popup_scroll::click(&mut h, "Command argument 2", AccessibleRole::ListItem);
    let row = ui::popup_item(&h.window, "Command argument 2", AccessibleRole::ListItem);
    let rect = row.geometry();
    let origin = row.map_to_native_window(rect.origin);
    drop(row); // The test must not retain a removed delegate through its ItemRc.
    h.window.window().dispatch_event(WindowEvent::PointerMoved {
        position: slint::LogicalPosition::new(origin.x + 30.0, origin.y + rect.size.height / 2.0),
    });
    std::thread::sleep(std::time::Duration::from_millis(650));
    slint::platform::update_timers_and_animations();
    h.draw();
    let tooltips = WindowInner::from_pub(h.window.window())
        .active_popups()
        .iter()
        .filter(|popup| matches!(popup.window_kind, WindowKind::ToolTip))
        .count();
    assert!(
        tooltips > 0,
        "actual hovered argument tooltip must be materialized"
    );
    // Activate the real header control without moving the pointer away from the
    // delegate being removed, keeping its live tooltip in the regression path.
    ui::popup_item(&h.window, "Remove argument", AccessibleRole::Button)
        .accessible_action(&AccessibilityAction::Default);
    h.draw();
    assert_eq!(
        h.window
            .global::<UserCommandSettingsView>()
            .get_argument_rows()
            .row_count(),
        1
    );
    assert_eq!(
        h.window
            .global::<UserCommandSettingsView>()
            .get_argument_rows()
            .row_data(0)
            .unwrap(),
        "--literal flag"
    );
    assert!(h.window.get_options_open());
    assert_eq!(
        ui::popup_item(&h.window, "Command argument 1", AccessibleRole::TextInput)
            .accessible_string_property(AccessibleStringProperty::Value)
            .as_deref(),
        Some("--literal flag")
    );
}
