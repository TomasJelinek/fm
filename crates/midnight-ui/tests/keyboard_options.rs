#[allow(dead_code)]
#[path = "support/modeless.rs"]
mod modeless;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{
    item_tree::ItemRc,
    items::AccessibleRole,
    window::{WindowInner, WindowKind},
};
use midnight_core::*;
use midnight_ui::{KeyboardEditorView, Presenter, install, refresh};
use slint::{
    ComponentHandle, Model,
    platform::{Key, WindowAdapter, WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::{cell::RefCell, rc::Rc, sync::Arc};

struct Harness {
    ui: ui::TestUi,
    presenter: Rc<RefCell<Presenter>>,
    commands: Rc<RefCell<Vec<Command>>>,
}

type NativeWindows = Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>;

fn native_help_harness(bindings: &[(&str, &[&str])]) -> (Harness, NativeWindows) {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    let window = midnight_ui::MainWindow::new().unwrap();
    window.set_preferences_ready(true);
    midnight_ui::install_file_keys(&window);
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let native = windows.borrow()[0].clone();
    let mut test = ui::TestUi {
        window,
        native,
        pixels: Vec::new(),
        width: 1340,
        height: 820,
    };
    test.resize(1340, 820);
    (Harness::with_ui(bindings, Vec::new(), test), windows)
}

fn native_help(
    windows: &NativeWindows,
    main: &midnight_ui::MainWindow,
    snapshot: &str,
) -> Rc<MinimalSoftwareWindow> {
    assert_eq!(
        windows.borrow().len(),
        2,
        "Help creates its own window adapter"
    );
    assert!(
        !ui::has_popup(main),
        "Help must not open a main-window popup"
    );
    let child = windows.borrow()[1].clone();
    assert!(child.window().is_visible());
    modeless::draw(&child, 740, 600, snapshot);
    child
}

#[test]
fn first_search_character_retires_hovered_command_tooltip_before_removing_rows() {
    let mut h = Harness::new(&[]);
    h.open_keyboard();
    let window = h.ui.window.clone_strong();
    let view = window.global::<KeyboardEditorView>();
    view.invoke_move_selection(1_000_000);
    h.draw();
    ui::click_popup(&window, "Search commands", AccessibleRole::TextInput);
    let hovered = {
        let row = ui::popup_item(
            &window,
            view.get_selected_label().as_str(),
            AccessibleRole::ListItem,
        );
        let rect = row.geometry();
        let origin = row.map_to_native_window(rect.origin);
        slint::LogicalPosition::new(
            origin.x + rect.size.width / 2.,
            origin.y + rect.size.height / 2.,
        )
    }; // Release the row handle: the filter must actually destroy its delegate.
    window
        .window()
        .dispatch_event(slint::platform::WindowEvent::PointerMoved { position: hovered });
    std::thread::sleep(std::time::Duration::from_millis(650));
    slint::platform::update_timers_and_animations();
    h.draw();
    let tooltip_count = || {
        WindowInner::from_pub(window.window())
            .active_popups()
            .iter()
            .filter(|popup| matches!(popup.window_kind, WindowKind::ToolTip))
            .count()
    };
    assert!(
        tooltip_count() > 0,
        "the command-row tooltip must be visible before typing"
    );
    let selected = view.get_selected_id();
    ui::key(&window, "x");
    h.draw();
    assert_eq!(view.get_filter(), "x");
    assert_eq!(view.get_selected_id(), selected);
    assert_eq!(view.get_selected_row(), -1);
    assert_eq!(tooltip_count(), 0);
    assert!(ui::has_popup(&window), "Options must remain open");
    ui::key(&window, Key::Backspace);
    h.draw();
    assert_eq!(view.get_filter(), "");
    assert_eq!(view.get_selected_id(), selected);
}

#[test]
fn typing_shortcut_search_survives_scrolled_selection_and_empty_results() {
    let mut h = Harness::new(&[]);
    h.open_keyboard();
    let window = h.ui.window.clone_strong();
    for query in [
        "viewer",
        "copy",
        "zzz no command",
        "",
        "rename",
        "žluťoučký",
        "files",
    ] {
        window
            .global::<KeyboardEditorView>()
            .invoke_move_selection(1000000);
        h.draw();
        ui::click_popup(&window, "Search commands", AccessibleRole::TextInput);
        ui::chord(&window, &[Key::Control], "a");
        ui::key(&window, Key::Backspace);
        h.draw();
        for character in query.chars() {
            ui::key(&window, character.to_string());
            h.draw();
        }
        assert_eq!(window.global::<KeyboardEditorView>().get_filter(), query);
        assert!(ui::has_popup(&window));
    }
}

#[test]
fn preview_options_persist_choices_and_clear_cache_without_closing_settings() {
    let mut h = Harness::new(&[]);
    let clears = Rc::new(std::cell::Cell::new(0));
    let observed = clears.clone();
    h.ui.window
        .on_thumbnail_clear_cache(move || observed.set(observed.get() + 1));
    h.ui.window.invoke_show_options();
    h.draw();
    ui::click_popup(&h.ui.window, "Previews", AccessibleRole::Button);
    h.draw();
    assert!(h.ui.window.get_thumbnails_enabled());
    assert!(!h.ui.window.get_thumbnails_network());
    ui::click_popup(
        &h.ui.window,
        "Allow previews on network locations",
        AccessibleRole::Checkbox,
    );
    h.draw();
    assert!(h.ui.window.get_thumbnails_network());
    ui::click_popup(
        &h.ui.window,
        "Show image and video previews",
        AccessibleRole::Checkbox,
    );
    h.draw();
    assert!(!h.ui.window.get_thumbnails_enabled());
    assert!(
        h.ui.window.get_thumbnails_network(),
        "disabling previews retains the separate network choice"
    );
    ui::click_popup(
        &h.ui.window,
        "Clear thumbnail cache",
        AccessibleRole::Button,
    );
    h.draw();
    assert_eq!(clears.get(), 1);
    assert!(ui::has_popup(&h.ui.window));
    assert!(
        h.commands
            .borrow()
            .iter()
            .all(|command| !matches!(command, Command::SavePreferences { .. }))
    );
    ui::click_popup(&h.ui.window, "OK", AccessibleRole::Button);
    assert!(h.commands.borrow().iter().any(|command| matches!(command,
        Command::SavePreferences { preferences, .. }
            if !preferences.thumbnails_enabled && preferences.thumbnails_network
    )));
}

#[test]
fn shortcut_editor_filters_commands_and_captures_without_dispatching_actions() {
    let mut h = Harness::new(&[]);
    h.open_keyboard();
    ui::click_popup(&h.ui.window, "Search commands", AccessibleRole::TextInput);
    ui::key(&h.ui.window, "files.copy");
    h.draw();
    ui::click_popup(
        &h.ui.window,
        "Copy files to clipboard",
        AccessibleRole::ListItem,
    );
    ui::click_popup(
        &h.ui.window,
        "Press shortcut keys",
        AccessibleRole::TextInput,
    );
    h.commands.borrow_mut().clear();
    ui::key(&h.ui.window, Key::F8);
    assert!(
        h.commands.borrow().is_empty(),
        "capture must not delete or save"
    );
    for key in [Key::F4, Key::Return, Key::F1] {
        ui::key(&h.ui.window, key);
        assert!(h.commands.borrow().is_empty());
        ui::popup_item(
            &h.ui.window,
            "Press shortcut keys",
            AccessibleRole::TextInput,
        );
    }
    ui::chord(&h.ui.window, &[Key::Control, Key::Alt], "y");
    assert!(h.commands.borrow().is_empty());
    ui::click_popup(&h.ui.window, "Assign", AccessibleRole::Button);
    h.draw();
    assert!(
        h.commands
            .borrow()
            .iter()
            .all(|command| !matches!(command, Command::SavePreferences { .. }))
    );
    ui::click_popup(&h.ui.window, "OK", AccessibleRole::Button);
    let commands = h.commands.borrow();
    let saved = commands
        .iter()
        .rev()
        .find_map(|command| match command {
            Command::SavePreferences { preferences, .. } => Some(preferences),
            _ => None,
        })
        .expect("OK saves the validated map asynchronously");
    let map = ShortcutMap::new(&saved.shortcuts).unwrap();
    assert_eq!(
        map.bindings("files.copy")
            .unwrap()
            .iter()
            .map(ShortcutChord::display)
            .collect::<Vec<_>>(),
        ["Ctrl+C", "Ctrl+Alt+Y"]
    );
}

#[test]
fn shortcut_capture_keeps_physical_digits_and_tab_escape_exit_without_dispatch() {
    let mut h = Harness::new(&[]);
    h.open_keyboard();
    h.select_action("files.copy");
    ui::click_popup(
        &h.ui.window,
        "Press shortcut keys",
        AccessibleRole::TextInput,
    );
    h.commands.borrow_mut().clear();
    assert!(
        h.ui.window
            .invoke_physical_shortcut("Digit7".into(), true, true, false, false)
    );
    assert_eq!(
        h.ui.window.global::<KeyboardEditorView>().get_candidate(),
        "Ctrl+Alt+Digit7"
    );
    h.ui.window.set_native_digit_event(true);
    ui::key(&h.ui.window, "/");
    h.ui.window.set_native_digit_event(false);
    assert_eq!(
        h.ui.window.global::<KeyboardEditorView>().get_candidate(),
        "Ctrl+Alt+Digit7"
    );
    h.ui.window.set_native_alt_gr(true);
    ui::key(&h.ui.window, "@");
    h.ui.window.set_native_alt_gr(false);
    assert_eq!(
        h.ui.window.global::<KeyboardEditorView>().get_candidate(),
        "Ctrl+Alt+Digit7"
    );
    for (key, expected) in [(Key::Tab, "Tab"), (Key::Escape, "Escape")] {
        ui::click_popup(
            &h.ui.window,
            "Press shortcut keys",
            AccessibleRole::TextInput,
        );
        ui::key(&h.ui.window, key);
        h.draw();
        assert_eq!(
            h.ui.window.global::<KeyboardEditorView>().get_candidate(),
            expected
        );
        assert!(!h.ui.window.global::<KeyboardEditorView>().get_capturing());
        assert!(ui::has_popup(&h.ui.window));
        assert!(h.commands.borrow().is_empty());
    }
}

#[test]
fn settings_navigation_and_footer_fit_both_densities_and_large_fonts() {
    let mut h = Harness::new(&[]);
    h.open_keyboard();
    h.ui.resize(1040, 720);
    for (theme, density, font) in [(1, 0, 12), (2, 0, 12), (1, 1, 14), (2, 1, 32)] {
        h.ui.window.invoke_apply_preferences(theme, density);
        h.ui.window.invoke_apply_fonts("".into(), font, font);
        std::thread::sleep(std::time::Duration::from_millis(220));
        for _ in 0..2 {
            slint::platform::update_timers_and_animations();
            h.ui.draw();
        }
        for topic in ["Appearance", "Keyboard"] {
            h.ui.options_topic(if topic == "Appearance" { 1 } else { 3 });
            for _ in 0..2 {
                slint::platform::update_timers_and_animations();
                h.ui.draw();
            }
            let done = ui::popup_item(&h.ui.window, "Cancel", AccessibleRole::Button);
            let rect = done.geometry();
            let position = done.map_to_native_window(rect.origin);
            assert!(position.x >= 0.0 && position.y >= 0.0);
            assert!(position.x + rect.size.width <= h.ui.width as f32);
            assert!(position.y + rect.size.height <= h.ui.height as f32);
            modeless::bitmap::snapshot(
                &h.ui.pixels,
                h.ui.width,
                h.ui.height,
                &format!("settings-{topic}-{theme}-{density}-{font}"),
            );
        }
    }
}

#[test]
fn topic_arrow_navigation_moves_focus_with_selection() {
    let mut h = Harness::new(&[]);
    h.open_keyboard();
    ui::click_popup(&h.ui.window, "General", AccessibleRole::Button);
    ui::key(&h.ui.window, Key::DownArrow);
    h.draw();
    assert_eq!(h.ui.window.get_options_topic_index(), 1);
    ui::key(&h.ui.window, Key::Return);
    h.draw();
    assert_eq!(h.ui.window.get_options_topic_index(), 1);
}

impl Harness {
    fn commit(&mut self) {
        ui::click_popup(&self.ui.window, "OK", AccessibleRole::Button);
        let revision = self
            .commands
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::SavePreferences { revision, .. } => Some(*revision),
                _ => None,
            })
            .unwrap();
        self.presenter.borrow_mut().apply(Event::PreferencesSaved {
            revision,
            result: Ok(()),
        });
        refresh(&self.ui.window, &self.presenter.borrow());
        self.draw();
    }
    fn new(bindings: &[(&str, &[&str])]) -> Self {
        Self::with_bookmarks(bindings, Vec::new())
    }

    fn with_bookmarks(bindings: &[(&str, &[&str])], bookmarks: Vec<Bookmark>) -> Self {
        let ui = ui::TestUi::new();
        Self::with_ui(bindings, bookmarks, ui)
    }

    fn with_ui(bindings: &[(&str, &[&str])], bookmarks: Vec<Bookmark>, ui: ui::TestUi) -> Self {
        let base = std::env::temp_dir().join("shortcut UI 文档");
        let presenter = Rc::new(RefCell::new(Presenter::new(
            base.clone(),
            base.join("other"),
        )));
        let commands = Rc::new(RefCell::new(Vec::new()));
        let sent = commands.clone();
        install(
            &ui.window,
            presenter.clone(),
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
        let entries: Arc<[Entry]> = vec![Entry::new(
            EntryId(1),
            base.join("file.txt"),
            EntryKind::File,
        )]
        .into();
        presenter.borrow_mut().apply(Event::ListingFinished {
            pane: PaneId::Left,
            navigation,
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
        });
        let values = UserPreferences {
            bookmarks,
            shortcuts: bindings
                .iter()
                .map(|(action, keys)| ShortcutBinding {
                    action: (*action).into(),
                    keys: keys.iter().map(|key| (*key).into()).collect(),
                })
                .collect(),
            ..Default::default()
        };
        values.validate().unwrap();
        presenter.borrow_mut().apply_with_sender(
            Event::PreferencesLoaded(Ok(values.into())),
            &|command| {
                commands.borrow_mut().push(command);
                true
            },
        );
        let mut h = Self {
            ui,
            presenter,
            commands,
        };
        h.draw();
        ui::click(
            &h.ui.window,
            h.ui.window.get_left_list_left() + 100.0,
            h.ui.window.get_left_list_top() + 1.5 * h.ui.window.get_left_view_row_height(),
        );
        h.commands.borrow_mut().clear();
        h
    }

    fn draw(&mut self) {
        refresh(&self.ui.window, &self.presenter.borrow());
        slint::platform::update_timers_and_animations();
        self.ui.draw();
    }

    fn open_keyboard(&mut self) {
        self.ui.window.invoke_show_options();
        self.draw();
        ui::click_popup(&self.ui.window, "Keyboard", AccessibleRole::Button);
        self.draw();
    }

    fn select_action(&mut self, id: &str) {
        let action = shortcut_catalog()
            .iter()
            .find(|action| action.id == id)
            .unwrap();
        ui::click_popup(
            &self.ui.window,
            "Search commands",
            AccessibleRole::TextInput,
        );
        ui::chord(&self.ui.window, &[Key::Control], "a");
        ui::key(&self.ui.window, id);
        self.draw();
        ui::click_popup(&self.ui.window, &action.label, AccessibleRole::ListItem);
        self.draw();
    }

    fn edit(&mut self, value: &str) {
        ui::click_popup(
            &self.ui.window,
            "Press shortcut keys",
            AccessibleRole::TextInput,
        );
        let chord = ShortcutChord::parse(value).unwrap();
        let modifiers = [
            (chord.control, Key::Control),
            (chord.alt, Key::Alt),
            (chord.shift, Key::Shift),
            (chord.meta, Key::Meta),
        ]
        .into_iter()
        .filter_map(|(pressed, key)| pressed.then_some(key))
        .collect::<Vec<_>>();
        ui::chord(&self.ui.window, &modifiers, chord.key.as_str());
        self.draw();
    }
}

#[test]
fn numbered_hot_paths_activate_from_both_popups_and_keep_the_origin_pane() {
    let destination = std::env::temp_dir().join("numbered saved path 文档");
    let mut h = Harness::with_bookmarks(
        &[],
        vec![Bookmark {
            network: None,
            archive: None,
            id: BookmarkId(1),
            name: "Saved".into(),
            path: destination.clone(),
            slot: Some(10),
        }],
    );
    h.ui.window.invoke_open_volume_menu(1);
    h.draw();
    assert_eq!(h.ui.window.get_volume_popup_pane(), 1);
    assert!(
        h.ui.window
            .invoke_physical_shortcut("Digit0".into(), false, false, false, false,)
    );
    assert!(!ui::has_popup(&h.ui.window));
    assert!(h.commands.borrow().iter().any(|command| matches!(command,
        Command::Navigate { pane: PaneId::Right, path, .. } if path == &destination)));
    h.commands.borrow_mut().clear();
    // Hot Paths is shared and follows the active panel. Select its intended
    // owner explicitly after the right panel's volume popup activated that pane.
    h.ui.window.invoke_activate(0);
    h.draw();
    ui::click_root(&h.ui.window, "Hot Paths", AccessibleRole::Combobox);
    h.draw();
    assert_eq!(h.ui.window.get_hot_path_popup_pane(), 0);
    ui::key(&h.ui.window, "0");
    assert!(!ui::has_popup(&h.ui.window));
    assert!(h.commands.borrow().iter().any(|command| matches!(command,
        Command::Navigate { pane: PaneId::Left, path, .. } if path == &destination)));
}

#[test]
fn popup_hot_paths_honor_remapping_other_panel_and_unassigned_slots() {
    let destination = std::env::temp_dir().join("mapped saved path");
    let mut h = Harness::with_bookmarks(
        &[("bookmark.open.1", &["Ctrl+Alt+Y"])],
        vec![Bookmark {
            network: None,
            archive: None,
            id: BookmarkId(1),
            name: "Saved".into(),
            path: destination.clone(),
            slot: Some(1),
        }],
    );
    ui::click_root(&h.ui.window, "Hot Paths", AccessibleRole::Combobox);
    h.draw();
    for digit in ["1", "2"] {
        ui::key(&h.ui.window, digit);
        assert!(h.commands.borrow().is_empty());
        assert!(ui::has_popup(&h.ui.window));
    }
    h.ui.window.set_native_alt_gr(true);
    assert!(
        !h.ui
            .window
            .invoke_physical_shortcut("Digit1".into(), true, true, false, false)
    );
    h.ui.window.set_native_alt_gr(false);
    ui::chord(&h.ui.window, &[Key::Control, Key::Alt], "y");
    assert!(!ui::has_popup(&h.ui.window));
    assert!(h.commands.borrow().iter().any(|command| matches!(command,
        Command::Navigate { pane: PaneId::Left, path, .. } if path == &destination)));
    h.commands.borrow_mut().clear();
    h.ui.window.invoke_open_volume_menu(0);
    h.draw();
    assert!(
        h.ui.window
            .invoke_physical_shortcut("Digit1".into(), true, true, false, false)
    );
    assert!(!ui::has_popup(&h.ui.window));
    assert!(h.commands.borrow().iter().any(|command| matches!(command,
        Command::Navigate { pane: PaneId::Right, path, .. } if path == &destination)));
}

#[test]
fn popup_hot_paths_reject_replaced_choices_and_preserve_volume_binding_priority() {
    let destination = std::env::temp_dir().join("captured saved path");
    let mut h = Harness::with_bookmarks(
        &[("panel.as-other", &["Digit1"])],
        vec![Bookmark {
            network: None,
            archive: None,
            id: BookmarkId(1),
            name: "Saved".into(),
            path: destination.clone(),
            slot: Some(1),
        }],
    );
    h.ui.window.invoke_open_volume_menu(0);
    h.draw();
    assert!(
        h.ui.window
            .invoke_physical_shortcut("Digit1".into(), false, false, false, false)
    );
    assert!(!ui::has_popup(&h.ui.window));
    assert!(h.commands.borrow().iter().any(|command| matches!(command,
        Command::Navigate { pane: PaneId::Left, path, .. } if path != &destination)));
    h.commands.borrow_mut().clear();
    ui::click_root(&h.ui.window, "Hot Paths", AccessibleRole::Combobox);
    h.draw();
    // A settings update can replace identities while the popup stays open.
    h.presenter
        .borrow_mut()
        .rename_bookmark(BookmarkId(1), "Changed".into(), &|_| true);
    h.draw();
    ui::chord(&h.ui.window, &[Key::Control], "1");
    assert!(
        h.commands.borrow().is_empty(),
        "old menu cannot target the replacement row"
    );
}

#[test]
fn persisted_file_shortcut_replaces_old_alias_and_preserves_native_text_copy() {
    let h = Harness::new(&[("files.copy", &["Ctrl+Alt+Y"])]);
    ui::chord(&h.ui.window, &[Key::Control], "c");
    assert!(
        h.commands
            .borrow()
            .iter()
            .all(|command| !matches!(command, Command::CopyFiles { .. })),
        "old Ctrl+C must be removed when Copy is reassigned"
    );
    h.commands.borrow_mut().clear();
    ui::chord(&h.ui.window, &[Key::Control, Key::Alt], "y");
    assert!(matches!(
        h.commands.borrow().last(),
        Some(Command::CopyFiles { .. })
    ));
    h.commands.borrow_mut().clear();
    h.ui.window.invoke_set_command("native text".into(), 11);
    ui::chord(&h.ui.window, &[Key::Control], "a");
    ui::chord(&h.ui.window, &[Key::Control], "c");
    ui::chord(&h.ui.window, &[Key::Control], "v");
    assert!(h.commands.borrow().is_empty());
}

#[test]
fn remapped_command_execute_removes_the_native_enter_acceptance_path() {
    let h = Harness::new(&[("command.execute", &["Ctrl+Alt+Enter"])]);
    let submitted = Rc::new(RefCell::new(Vec::new()));
    let observed = submitted.clone();
    h.ui.window.on_submit_command(move |text| {
        observed.borrow_mut().push(text);
        true
    });
    h.ui.window.invoke_set_command("echo literal".into(), 12);
    assert!(h.ui.window.get_command_focused());
    ui::key(&h.ui.window, Key::Return);
    assert!(
        submitted.borrow().is_empty(),
        "unbound Enter must not execute via accepted()"
    );
    assert_eq!(h.ui.window.get_command_text(), "echo literal");
    ui::chord(&h.ui.window, &[Key::Control, Key::Alt], Key::Return);
    assert_eq!(
        submitted.borrow().as_slice(),
        &[slint::SharedString::from("echo literal")]
    );
    assert!(h.ui.window.get_command_text().is_empty());
}

#[test]
fn disabled_open_and_command_execute_have_no_default_key_bypass() {
    let h = Harness::new(&[("panel.open", &[]), ("command.execute", &[])]);
    ui::key(&h.ui.window, Key::Return);
    assert!(h.commands.borrow().is_empty());
    let submitted = Rc::new(RefCell::new(Vec::new()));
    let observed = submitted.clone();
    h.ui.window.on_submit_command(move |text| {
        observed.borrow_mut().push(text);
        true
    });
    h.ui.window.invoke_set_command("retained draft".into(), 14);
    ui::key(&h.ui.window, Key::Return);
    assert!(submitted.borrow().is_empty());
    assert_eq!(h.ui.window.get_command_text(), "retained draft");
}

#[test]
fn volume_popup_uses_configured_mnemonic_and_keeps_widget_escape() {
    let mut h = Harness::new(&[("panel.as-other", &["Ctrl+Alt+O"])]);
    let destination = std::env::temp_dir().join("shortcut UI 文档").join("other");
    ui::chord(&h.ui.window, &[Key::Alt], Key::F1);
    h.draw();
    assert_eq!(h.ui.window.get_volume_popup_pane(), 0);
    ui::key(&h.ui.window, ".");
    assert!(ui::has_popup(&h.ui.window), "the old mnemonic is disabled");
    assert!(h.commands.borrow().is_empty());
    ui::chord(&h.ui.window, &[Key::Control, Key::Alt], "o");
    assert!(h.commands.borrow().iter().any(|command| matches!(command,
        Command::Navigate { pane: PaneId::Left, path, .. } if path == &destination)));
    assert!(!ui::has_popup(&h.ui.window));
    h.commands.borrow_mut().clear();
    ui::chord(&h.ui.window, &[Key::Alt], Key::F1);
    ui::key(&h.ui.window, Key::Escape);
    assert!(!ui::has_popup(&h.ui.window));
    assert!(h.commands.borrow().is_empty());
}

#[test]
fn global_help_binding_changes_without_stealing_editor_keys() {
    let (mut h, windows) = native_help_harness(&[("app.help", &["Ctrl+Alt+H"])]);
    h.ui.window.invoke_set_command("editable".into(), 8);
    ui::key(&h.ui.window, Key::F1);
    assert!(
        !ui::has_popup(&h.ui.window),
        "old global key must be disabled"
    );
    assert_eq!(
        windows.borrow().len(),
        1,
        "old global key must not create Help"
    );
    ui::chord(&h.ui.window, &[Key::Control, Key::Alt], "h");
    h.draw();
    native_help(&windows, &h.ui.window, "keyboard-help-remapped-global");
    assert_eq!(h.ui.window.get_command_text(), "editable");
}

#[test]
fn reassigned_clipboard_chords_remain_native_while_searching_filenames() {
    let mut h = Harness::new(&[
        ("files.copy", &[]),
        ("files.paste", &[]),
        ("files.rename", &["Ctrl+C"]),
        ("files.delete", &["Ctrl+V"]),
    ]);
    h.open_keyboard();
    h.select_action("files.rename");
    assert_eq!(
        h.ui.window.get_keyboard_scope(),
        "File list outside filename search"
    );
    ui::click_popup(&h.ui.window, "Cancel", AccessibleRole::Button);
    h.ui.window.invoke_focus_files();
    let mutations = Rc::new(RefCell::new(Vec::new()));
    let observed = mutations.clone();
    h.ui.window.on_mutation(move |pane, action| {
        observed.borrow_mut().push((pane, action));
    });
    ui::key(&h.ui.window, "file");
    h.draw();
    assert!(h.ui.window.get_left_search_active());
    ui::chord(&h.ui.window, &[Key::Control], "c");
    ui::chord(&h.ui.window, &[Key::Control], "v");
    assert!(
        mutations.borrow().is_empty(),
        "search keeps native copy/paste"
    );
    assert_eq!(h.ui.window.get_left_search_query(), "file");
    ui::key(&h.ui.window, Key::Escape);
    assert!(!h.ui.window.get_left_search_active());
    ui::chord(&h.ui.window, &[Key::Control], "c");
    ui::chord(&h.ui.window, &[Key::Control], "v");
    assert_eq!(mutations.borrow().as_slice(), &[(0, 0), (0, 2)]);
}

#[test]
fn unbound_physical_digits_cannot_fall_back_to_configured_logical_punctuation() {
    let (mut h, windows) =
        native_help_harness(&[("bookmark.open.1", &[]), ("app.help", &["Ctrl+Plus"])]);
    h.ui.window.set_native_digit_event(true);
    ui::chord(&h.ui.window, &[Key::Control], "+");
    h.draw();
    assert!(
        !ui::has_popup(&h.ui.window),
        "unbound physical Digit1 cannot invoke logical Ctrl+Plus"
    );
    assert_eq!(windows.borrow().len(), 1);
    h.ui.window.set_native_digit_event(false);
    h.ui.window.set_native_alt_gr(true);
    ui::chord(&h.ui.window, &[Key::Control], "+");
    assert!(
        !ui::has_popup(&h.ui.window),
        "AltGr cannot invoke a configured application key"
    );
    assert_eq!(windows.borrow().len(), 1);
    h.ui.window.set_native_alt_gr(false);
    ui::chord(&h.ui.window, &[Key::Control], "+");
    h.draw();
    native_help(&windows, &h.ui.window, "keyboard-help-physical-digit");
}

#[test]
fn keyboard_options_validate_apply_disable_and_restore_effective_bindings() {
    let mut h = Harness::new(&[]);
    h.open_keyboard();
    h.select_action("files.copy");
    h.commands.borrow_mut().clear();
    h.edit("Ctrl+V");
    assert!(!h.ui.window.global::<KeyboardEditorView>().get_can_assign());
    assert!(
        h.ui.window
            .global::<KeyboardEditorView>()
            .get_used_by()
            .contains("Paste")
    );
    assert!(
        h.commands
            .borrow()
            .iter()
            .all(|command| !matches!(command, Command::SavePreferences { .. })),
        "a conflicting shortcut must not partly save"
    );
    h.edit("Ctrl+Alt+Y");
    ui::click_popup(&h.ui.window, "Assign", AccessibleRole::Button);
    h.draw();
    assert!(
        h.ui.window
            .global::<KeyboardEditorView>()
            .get_aliases()
            .iter()
            .any(|key| key == "Ctrl+Alt+Y")
    );
    assert!(
        h.commands
            .borrow()
            .iter()
            .all(|command| !matches!(command, Command::SavePreferences { .. }))
    );
    for label in [
        "Remove",
        "Remove",
        "Restore command",
        "Restore all defaults",
        "Reset all shortcuts",
    ] {
        ui::click_popup(&h.ui.window, label, AccessibleRole::Button);
        h.draw();
    }
    ui::click_popup(&h.ui.window, "Cancel", AccessibleRole::Button);
    h.ui.window.invoke_focus_files();
    h.commands.borrow_mut().clear();
    ui::chord(&h.ui.window, &[Key::Control], "c");
    assert!(matches!(
        h.commands.borrow().last(),
        Some(Command::CopyFiles { .. })
    ));
}

#[test]
fn in_session_reassignment_updates_the_bottom_caption_and_rendered_help() {
    let (mut h, windows) = native_help_harness(&[]);
    h.open_keyboard();
    h.select_action("files.rename");
    ui::click_popup(&h.ui.window, "Remove", AccessibleRole::Button);
    h.draw();
    h.edit("Ctrl+Alt+R");
    ui::click_popup(&h.ui.window, "Assign", AccessibleRole::Button);
    h.draw();
    h.commit();
    h.draw();
    ui::root_item(&h.ui.window, "F2 — unavailable", AccessibleRole::Button);
    let root_text = ui::text_values(ItemRc::new_root(
        WindowInner::from_pub(h.ui.window.window()).component(),
    ));
    assert!(!root_text.iter().any(|text| text == "F2 Rename"));
    assert!(!ui::has_popup(&h.ui.window), "OK closed Options");
    assert_eq!(
        h.ui.window.get_focused_pane(),
        0,
        "OK returns focus to the active panel"
    );
    ui::key(&h.ui.window, Key::F1);
    h.draw();
    let child = native_help(&windows, &h.ui.window, "keyboard-help-reassigned-rename");
    let inner = WindowInner::from_pub(child.window());
    let text = ui::text_values(ItemRc::new_root(inner.component()))
        .iter()
        .map(|text| text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("Rename focused item — Ctrl+Alt+R"));
    assert!(!text.contains("Rename focused item — F2"));
}

#[test]
fn restoring_unchanged_defaults_also_discards_the_unsaved_editor_draft() {
    let mut h = Harness::new(&[]);
    h.open_keyboard();
    h.select_action("files.copy");
    h.edit("Ctrl+Alt+Y");
    ui::click_popup(&h.ui.window, "Restore command", AccessibleRole::Button);
    h.draw();
    assert!(
        h.ui.window
            .global::<KeyboardEditorView>()
            .get_candidate()
            .is_empty()
    );
    assert!(!h.ui.window.global::<KeyboardEditorView>().get_can_assign());
    ui::click_popup(&h.ui.window, "OK", AccessibleRole::Button);
    let sent = h.commands.borrow();
    let saved = sent
        .iter()
        .rev()
        .find_map(|command| match command {
            Command::SavePreferences { preferences, .. } => Some(preferences),
            _ => None,
        })
        .expect("OK saves the restored default binding");
    assert_eq!(
        ShortcutMap::new(&saved.shortcuts)
            .unwrap()
            .bindings("files.copy")
            .unwrap()[0]
            .display(),
        "Ctrl+C"
    );
}

#[test]
fn remapped_clear_files_uses_effective_binding_and_preserves_editor_focus() {
    let h = Harness::new(&[("selection.clear-files", &["Ctrl+Alt+Y"])]);
    ui::chord(&h.ui.window, &[Key::Control], "d");
    assert!(
        matches!(h.commands.borrow().last(), Some(Command::Selection(request))
        if matches!(request.action, SelectionAction::Bulk(SelectionOperation::Clear)))
    );
    h.commands.borrow_mut().clear();
    ui::chord(&h.ui.window, &[Key::Control, Key::Alt], "y");
    assert!(
        matches!(h.commands.borrow().last(), Some(Command::Selection(request))
        if matches!(request.action, SelectionAction::Bulk(SelectionOperation::ClearFilesOnly)))
    );
    h.commands.borrow_mut().clear();
    h.ui.window.invoke_set_command("editable text".into(), 13);
    ui::chord(&h.ui.window, &[Key::Control], "d");
    ui::chord(&h.ui.window, &[Key::Control, Key::Shift], "d");
    ui::chord(&h.ui.window, &[Key::Control, Key::Alt], "y");
    assert!(
        h.commands.borrow().is_empty(),
        "text focus cannot clear panel marks"
    );
}

#[test]
fn saved_legacy_select_and_clear_bindings_keep_their_original_operations() {
    let h = Harness::new(&[
        ("selection.all", &["Ctrl+A"]),
        ("selection.clear", &["Ctrl+D"]),
    ]);
    for (key, operation) in [
        ("a", SelectionOperation::All),
        ("d", SelectionOperation::Clear),
    ] {
        h.commands.borrow_mut().clear();
        ui::chord(&h.ui.window, &[Key::Control], key);
        let sent = h.commands.borrow();
        let selections = sent
            .iter()
            .filter_map(|command| match command {
                Command::Selection(request) => Some(&request.action),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            selections.len(),
            1,
            "a saved chord dispatches exactly one action"
        );
        assert!(matches!(selections[0], SelectionAction::Bulk(actual) if *actual == operation));
    }
    h.commands.borrow_mut().clear();
    for key in ["a", "d"] {
        ui::chord(&h.ui.window, &[Key::Control, Key::Shift], key);
    }
    assert!(
        h.commands
            .borrow()
            .iter()
            .all(|command| !matches!(command, Command::Selection(_))),
        "explicit legacy bindings replace the new default aliases"
    );
}
