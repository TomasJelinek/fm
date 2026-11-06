#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{
    item_tree::ItemRc,
    items::AccessibleRole,
    window::{WindowInner, WindowKind},
};
use midnight_core::*;
use midnight_ui::{Presenter, install, refresh};
use slint::{ComponentHandle, platform::Key};
use std::{cell::RefCell, rc::Rc, sync::Arc};

struct Harness {
    ui: ui::TestUi,
    presenter: Rc<RefCell<Presenter>>,
    commands: Rc<RefCell<Vec<Command>>>,
}

impl Harness {
    fn new(bindings: &[(&str, &[&str])]) -> Self {
        let ui = ui::TestUi::new();
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
            Event::PreferencesLoaded(Ok(values)),
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
        let index = shortcut_catalog()
            .iter()
            .position(|action| action.id == id)
            .unwrap();
        ui::click_popup(&self.ui.window, "Shortcut action", AccessibleRole::Combobox);
        ui::key(&self.ui.window, Key::Home);
        for _ in 0..index {
            ui::key(&self.ui.window, Key::DownArrow);
        }
        ui::key(&self.ui.window, Key::Return);
        self.draw();
    }

    fn edit(&mut self, value: &str) {
        ui::click_popup(
            &self.ui.window,
            "Shortcut bindings",
            AccessibleRole::TextInput,
        );
        ui::chord(&self.ui.window, &[Key::Control], "a");
        ui::key(&self.ui.window, value);
    }
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
    let mut h = Harness::new(&[("app.help", &["Ctrl+Alt+H"])]);
    h.ui.window.invoke_set_command("editable".into(), 8);
    ui::key(&h.ui.window, Key::F1);
    assert!(
        !ui::has_popup(&h.ui.window),
        "old global key must be disabled"
    );
    ui::chord(&h.ui.window, &[Key::Control, Key::Alt], "h");
    h.draw();
    assert!(ui::has_popup(&h.ui.window));
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
    ui::click_popup(&h.ui.window, "Done", AccessibleRole::Button);
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
    let mut h = Harness::new(&[("bookmark.open.1", &[]), ("app.help", &["Ctrl+Plus"])]);
    h.ui.window.set_native_digit_event(true);
    ui::chord(&h.ui.window, &[Key::Control], "+");
    h.draw();
    assert!(
        !ui::has_popup(&h.ui.window),
        "unbound physical Digit1 cannot invoke logical Ctrl+Plus"
    );
    h.ui.window.set_native_digit_event(false);
    h.ui.window.set_native_alt_gr(true);
    ui::chord(&h.ui.window, &[Key::Control], "+");
    assert!(
        !ui::has_popup(&h.ui.window),
        "AltGr cannot invoke a configured application key"
    );
    h.ui.window.set_native_alt_gr(false);
    ui::chord(&h.ui.window, &[Key::Control], "+");
    h.draw();
    assert!(ui::has_popup(&h.ui.window));
}

#[test]
fn keyboard_options_validate_apply_disable_and_restore_effective_bindings() {
    let mut h = Harness::new(&[]);
    h.open_keyboard();
    h.select_action("files.copy");
    h.commands.borrow_mut().clear();
    h.edit("Ctrl+V");
    ui::click_popup(&h.ui.window, "Apply shortcut", AccessibleRole::Button);
    assert!(
        h.commands
            .borrow()
            .iter()
            .all(|command| !matches!(command, Command::SavePreferences { .. })),
        "a conflicting shortcut must not partly save"
    );
    h.edit("Ctrl+Alt+Y");
    ui::click_popup(&h.ui.window, "Apply shortcut", AccessibleRole::Button);
    h.draw();
    {
        let sent = h.commands.borrow();
        let saved = sent
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::SavePreferences { preferences, .. } => Some(preferences),
                _ => None,
            })
            .expect("Apply must use asynchronous preference persistence");
        assert_eq!(
            ShortcutMap::new(&saved.shortcuts)
                .unwrap()
                .bindings("files.copy")
                .unwrap()[0]
                .display(),
            "Ctrl+Alt+Y"
        );
    }
    for label in [
        "Disable shortcut",
        "Restore default",
        "Restore all shortcuts",
    ] {
        ui::click_popup(&h.ui.window, label, AccessibleRole::Button);
        h.draw();
    }
    ui::click_popup(&h.ui.window, "Done", AccessibleRole::Button);
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
    let mut h = Harness::new(&[]);
    h.open_keyboard();
    h.select_action("files.rename");
    h.edit("Ctrl+Alt+R");
    ui::click_popup(&h.ui.window, "Apply shortcut", AccessibleRole::Button);
    h.draw();
    ui::click_popup(&h.ui.window, "Done", AccessibleRole::Button);
    h.draw();
    ui::root_item(&h.ui.window, "Ctrl+Alt+R Rename", AccessibleRole::Button);
    let root_text = ui::text_values(ItemRc::new_root(
        WindowInner::from_pub(h.ui.window.window()).component(),
    ));
    assert!(!root_text.iter().any(|text| text == "F2 Rename"));
    ui::key(&h.ui.window, Key::F1);
    h.draw();
    let inner = WindowInner::from_pub(h.ui.window.window());
    let popups = inner.active_popups();
    let popup = popups
        .iter()
        .rev()
        .find(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
        .unwrap();
    let text = ui::text_values(ItemRc::new_root(popup.component.clone()))
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
    ui::click_popup(&h.ui.window, "Restore default", AccessibleRole::Button);
    h.draw();
    ui::click_popup(&h.ui.window, "Apply shortcut", AccessibleRole::Button);
    let sent = h.commands.borrow();
    let saved = sent
        .iter()
        .rev()
        .find_map(|command| match command {
            Command::SavePreferences { preferences, .. } => Some(preferences),
            _ => None,
        })
        .expect("Apply saves the restored binding");
    assert_eq!(
        ShortcutMap::new(&saved.shortcuts)
            .unwrap()
            .bindings("files.copy")
            .unwrap()[0]
            .display(),
        "Ctrl+C"
    );
}
