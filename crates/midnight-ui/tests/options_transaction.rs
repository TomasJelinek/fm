//! Options previews are reversible and only OK commits the edit session.
#[path = "support/popup_scroll.rs"]
mod popup_scroll;
#[path = "support/render.rs"]
mod render;
use crate::support::ui;

use i_slint_core::items::AccessibleRole;
use midnight_core::*;
use midnight_ui::{ColorOptionsView, EditorOptionsView, Presenter, install, refresh};
use slint::{ComponentHandle, platform::Key};
use std::{cell::RefCell, rc::Rc};

type AppearanceNotice = (i32, i32, String, f32);

fn observe_appearance(window: &midnight_ui::MainWindow) -> Rc<RefCell<Vec<AppearanceNotice>>> {
    let observations = Rc::new(RefCell::new(Vec::new()));
    let output = observations.clone();
    let weak = window.as_weak();
    window.on_refresh_content_appearance(move || {
        if let Some(window) = weak.upgrade() {
            let theme = window.global::<midnight_ui::Theme>();
            output.borrow_mut().push((
                theme.get_scheme(),
                theme.get_density(),
                theme.get_effective_font_family().to_string(),
                theme.get_font_size(),
            ));
        }
    });
    observations
}

struct Harness {
    ui: ui::TestUi,
    presenter: Rc<RefCell<Presenter>>,
    sent: Rc<RefCell<Vec<Command>>>,
}
impl Harness {
    fn new() -> Self {
        Self::with_preferences(UserPreferences::default())
    }
    fn with_preferences(preferences: UserPreferences) -> Self {
        let ui = ui::TestUi::new();
        let presenter = Rc::new(RefCell::new(Presenter::new("left".into(), "right".into())));
        let sent = Rc::new(RefCell::new(Vec::new()));
        let captured = sent.clone();
        install(
            &ui.window,
            presenter.clone(),
            Rc::new(move |command| {
                captured.borrow_mut().push(command);
                true
            }),
        );
        presenter
            .borrow_mut()
            .apply(Event::PreferencesLoaded(Ok(Box::new(
                midnight_core::AcceptedPreferences {
                    preferences,
                    comparators: midnight_core::FileComparatorSnapshot::default(),
                },
            ))));
        refresh(&ui.window, &presenter.borrow());
        Self {
            ui,
            presenter,
            sent,
        }
    }
    fn saves(&self) -> Vec<(PreferencesRevision, UserPreferences)> {
        self.sent
            .borrow()
            .iter()
            .filter_map(|command| match command {
                Command::SavePreferences {
                    revision,
                    preferences,
                } => Some((*revision, *preferences.clone())),
                _ => None,
            })
            .collect()
    }
    fn event(&mut self, event: Event) {
        let sent = self.sent.clone();
        self.presenter
            .borrow_mut()
            .apply_with_sender(event, &|command| {
                sent.borrow_mut().push(command);
                true
            });
        refresh(&self.ui.window, &self.presenter.borrow());
        self.ui.draw();
    }
}

fn disk_io_choice(h: &mut Harness, index: i32) {
    let current = h.ui.window.get_disk_io_concurrency_index();
    popup_scroll::click(&mut h.ui, "Disk I/O concurrency", AccessibleRole::Combobox);
    let direction = if index >= current {
        Key::DownArrow
    } else {
        Key::UpArrow
    };
    for _ in 0..(index - current).abs() {
        ui::key(&h.ui.window, direction);
    }
    ui::key(&h.ui.window, Key::Return);
    h.ui.draw();
}

#[test]
fn disk_io_general_picker_defaults_to_automatic_and_only_ok_saves_manual_boundaries() {
    let mut h = Harness::new();
    assert_eq!(h.ui.window.get_disk_io_concurrency_index(), 0);
    for (index, save) in [(1, false), (32, true)] {
        h.ui.window.set_options_topic_index(0);
        h.ui.window.invoke_show_options();
        h.ui.draw();
        disk_io_choice(&mut h, index);
        assert_eq!(h.ui.window.get_disk_io_concurrency_index(), index);
        assert!(
            h.saves().is_empty(),
            "editing cannot persist a disk-I/O limit"
        );
        ui::click_popup(
            &h.ui.window,
            if save { "OK" } else { "Cancel" },
            AccessibleRole::Button,
        );
        h.ui.draw();
        if save {
            let writes = h.saves();
            assert_eq!(writes.len(), 1);
            assert_eq!(
                writes[0].1.disk_io_concurrency,
                DiskIoConcurrency::Manual(32)
            );
            h.event(Event::PreferencesSaved {
                revision: writes[0].0,
                result: Ok(midnight_core::FileComparatorSnapshot::default()),
            });
            assert!(!ui::has_popup(&h.ui.window));
        } else {
            assert_eq!(h.ui.window.get_disk_io_concurrency_index(), 0);
            assert!(h.saves().is_empty());
        }
    }
    h.ui.window.invoke_show_options();
    h.ui.draw();
    disk_io_choice(&mut h, 0);
    assert_eq!(h.saves().len(), 1);
    ui::click_popup(&h.ui.window, "OK", AccessibleRole::Button);
    let writes = h.saves();
    assert_eq!(writes.len(), 2);
    assert_eq!(
        writes[1].1.disk_io_concurrency,
        DiskIoConcurrency::Automatic
    );
}

#[test]
fn disk_io_general_picker_keeps_a_failed_save_draft_and_cancel_restores_automatic() {
    let mut h = Harness::new();
    let appearances = observe_appearance(&h.ui.window);
    h.ui.resize(320, 600);
    h.ui.window.invoke_show_options();
    h.ui.window.invoke_preferences_changed(24, 0, 0);
    h.ui.draw();
    disk_io_choice(&mut h, 1);
    ui::click_popup(&h.ui.window, "OK", AccessibleRole::Button);
    let writes = h.saves();
    assert_eq!(
        writes[0].1.disk_io_concurrency,
        DiskIoConcurrency::Manual(1)
    );
    h.event(Event::PreferencesSaved {
        revision: writes[0].0,
        result: Err(FsError::from_io(
            "save",
            std::path::Path::new("settings"),
            std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        )
        .into()),
    });
    assert!(ui::has_popup(&h.ui.window));
    assert_eq!(h.ui.window.get_disk_io_concurrency_index(), 1);
    assert_eq!(appearances.borrow().last().unwrap().0, 24);
    ui::click_popup(&h.ui.window, "Cancel", AccessibleRole::Button);
    h.ui.draw();
    assert_eq!(h.ui.window.get_disk_io_concurrency_index(), 0);
    assert_eq!(appearances.borrow().last().unwrap().0, 0);
    assert_eq!(h.saves().len(), 1);
}

#[test]
fn panel_history_changes_are_staged_until_ok_and_cancel_restores_them() {
    use slint::Model;
    let preferences = UserPreferences {
        remember_command_history: true,
        command_history: vec![CommandHistoryEntry {
            command: "echo žluťoučký 文档".into(),
            shell: ShellFlavor::PowerShell,
        }],
        ..Default::default()
    };
    let mut h = Harness::with_preferences(preferences);
    for save in [false, true] {
        h.ui.window.set_options_topic_index(0);
        h.ui.window.invoke_show_options();
        h.ui.draw();
        assert_eq!(h.ui.window.get_command_history().row_count(), 1);
        assert!(h.ui.window.get_remember_command_history());
        ui::click_popup(
            &h.ui.window,
            "Remember command history after restart",
            AccessibleRole::Checkbox,
        );
        ui::click_popup(
            &h.ui.window,
            "Clear command history",
            AccessibleRole::Button,
        );
        h.ui.draw();
        assert!(!h.ui.window.get_remember_command_history());
        assert_eq!(h.ui.window.get_command_history().row_count(), 0);
        assert!(h.saves().is_empty());
        ui::click_popup(
            &h.ui.window,
            if save { "OK" } else { "Cancel" },
            AccessibleRole::Button,
        );
        h.ui.draw();
        if save {
            let saves = h.saves();
            assert_eq!(saves.len(), 1);
            assert!(!saves[0].1.remember_command_history);
            assert!(saves[0].1.command_history.is_empty());
            h.event(Event::PreferencesSaved {
                revision: saves[0].0,
                result: Ok(midnight_core::FileComparatorSnapshot::default()),
            });
            assert!(!ui::has_popup(&h.ui.window));
        } else {
            assert!(h.saves().is_empty());
            assert!(h.ui.window.get_remember_command_history());
            assert_eq!(h.ui.window.get_command_history().row_count(), 1);
        }
    }
}

#[test]
fn checksum_option_is_visible_defaults_on_and_only_ok_persists_quick_mode() {
    let mut h = Harness::new();
    for save in [false, true] {
        h.ui.window.set_options_topic_index(0);
        h.ui.window.invoke_show_options();
        h.ui.draw();
        assert!(h.ui.window.get_copy_checksum_verification());
        ui::click_popup(
            &h.ui.window,
            "Verify entire copied file with SHA-256 (recommended)",
            AccessibleRole::Checkbox,
        );
        h.ui.draw();
        assert!(!h.ui.window.get_copy_checksum_verification());
        assert!(h.saves().is_empty());
        render::snapshot(&h.ui, "options-quick-verification");
        ui::click_popup(
            &h.ui.window,
            if save { "OK" } else { "Cancel" },
            AccessibleRole::Button,
        );
        h.ui.draw();
        if save {
            let writes = h.saves();
            assert_eq!(writes.len(), 1);
            assert!(!writes[0].1.copy_checksum_verification);
            h.event(Event::PreferencesSaved {
                revision: writes[0].0,
                result: Ok(midnight_core::FileComparatorSnapshot::default()),
            });
            assert!(!ui::has_popup(&h.ui.window));
        } else {
            assert!(h.ui.window.get_copy_checksum_verification());
            assert!(h.saves().is_empty());
        }
    }
}

#[test]
fn cancel_escape_and_x_restore_preview_without_writing_settings() {
    let mut h = Harness::new();
    let appearances = observe_appearance(&h.ui.window);
    for close in ["Cancel", "Escape", "Cancel dialog"] {
        h.ui.window.invoke_show_options();
        h.ui.window.invoke_preferences_changed(1, 0, 0);
        h.ui.window.invoke_font_options_changed(0, 18, 22, 1);
        h.ui.window.invoke_sizes_format_changed(true);
        h.ui.draw();
        render::snapshot(&h.ui, "options-preview");
        assert_eq!(h.ui.window.get_theme_index(), 1);
        assert_eq!(appearances.borrow().last().unwrap().0, 1);
        assert_eq!(appearances.borrow().last().unwrap().3, 18.0);
        assert!(h.saves().is_empty(), "preview must never reach disk");
        if close == "Escape" {
            ui::key(&h.ui.window, Key::Escape);
        } else {
            ui::click_popup(&h.ui.window, close, AccessibleRole::Button);
        }
        h.ui.draw();
        assert!(!ui::has_popup(&h.ui.window));
        assert_eq!(h.ui.window.get_theme_index(), 0);
        assert_eq!(h.ui.window.get_desktop_font_size(), 12);
        assert_eq!(appearances.borrow().last().unwrap().0, 0);
        assert_eq!(appearances.borrow().last().unwrap().3, 12.0);
        assert!(!h.ui.window.get_sizes_in_bytes());
        assert!(h.saves().is_empty());
    }
}

#[test]
fn reset_defaults_is_a_preview_and_cancel_restores_the_saved_options() {
    let preferences = UserPreferences {
        theme: PreferenceTheme::from_index(1).unwrap(),
        desktop_font_size: 18,
        quick_search_filter: true,
        size_display: SizeDisplay::Bytes,
        thumbnails_enabled: false,
        ..Default::default()
    };
    let mut h = Harness::with_preferences(preferences);
    let appearances = observe_appearance(&h.ui.window);
    h.ui.window.invoke_show_options();
    h.ui.draw();
    ui::click_popup(&h.ui.window, "Reset to defaults", AccessibleRole::Button);
    h.ui.draw();
    assert_eq!(
        h.ui.window.get_theme_index(),
        PreferenceTheme::default().index()
    );
    assert_eq!(h.ui.window.get_desktop_font_size(), 12);
    assert_eq!(appearances.borrow().last().unwrap().0, 0);
    assert_eq!(appearances.borrow().last().unwrap().3, 12.0);
    assert!(!h.ui.window.get_sizes_in_bytes());
    assert!(!h.ui.window.get_quick_search_filter());
    assert!(h.ui.window.get_thumbnails_enabled());
    render::snapshot(&h.ui, "options-reset-preview");
    assert!(h.saves().is_empty(), "Reset is not a save action");
    ui::click_popup(&h.ui.window, "Cancel", AccessibleRole::Button);
    h.ui.draw();
    assert_eq!(h.ui.window.get_theme_index(), 1);
    assert_eq!(h.ui.window.get_desktop_font_size(), 18);
    assert_eq!(appearances.borrow().last().unwrap().0, 1);
    assert_eq!(appearances.borrow().last().unwrap().3, 18.0);
    assert!(h.ui.window.get_sizes_in_bytes());
    assert!(h.ui.window.get_quick_search_filter());
    assert!(!h.ui.window.get_thumbnails_enabled());
    assert!(h.saves().is_empty());
}

#[test]
fn reset_defaults_button_is_reachable_in_the_smallest_options_window() {
    let mut h = Harness::new();
    h.ui.resize(320, 200);
    h.ui.window.invoke_show_options();
    h.ui.draw();
    for label in ["Reset to defaults", "OK", "Cancel"] {
        let item = ui::popup_item(&h.ui.window, label, AccessibleRole::Button);
        let rect = item.geometry();
        let origin = item.map_to_native_window(rect.origin);
        assert!(
            origin.x >= 0.0
                && origin.y >= 0.0
                && origin.x + rect.size.width <= 320.0
                && origin.y + rect.size.height <= 200.0,
            "{label}: {origin:?}/{rect:?}"
        );
    }
    ui::click_popup(&h.ui.window, "Reset to defaults", AccessibleRole::Button);
    h.ui.draw();
    render::snapshot(&h.ui, "options-reset-compact");
    assert!(h.saves().is_empty());
}

#[test]
fn ok_saves_once_and_closes_only_after_backend_success() {
    let mut h = Harness::new();
    let appearances = observe_appearance(&h.ui.window);
    h.ui.window.invoke_show_options();
    h.ui.window.invoke_preferences_changed(1, 0, 0);
    h.ui.draw();
    ui::key(&h.ui.window, Key::Return);
    h.ui.draw();
    let saves = h.saves();
    assert_eq!(saves.len(), 1);
    assert_eq!(saves[0].1.theme.index(), 1);
    assert!(
        ui::has_popup(&h.ui.window),
        "wait for the real storage result"
    );
    h.presenter.borrow_mut().apply(Event::PreferencesSaved {
        revision: saves[0].0,
        result: Ok(midnight_core::FileComparatorSnapshot::default()),
    });
    refresh(&h.ui.window, &h.presenter.borrow());
    h.ui.draw();
    assert!(!ui::has_popup(&h.ui.window));
    assert_eq!(h.ui.window.get_theme_index(), 1);
    assert_eq!(appearances.borrow().last().unwrap().0, 1);
    h.ui.window.invoke_show_options();
    h.ui.draw();
    ui::click_popup(&h.ui.window, "Cancel", AccessibleRole::Button);
    assert_eq!(h.ui.window.get_theme_index(), 1);
    assert_eq!(h.saves().len(), 1);
}

#[test]
fn ok_finds_invalid_editor_draft_on_another_page_and_cancel_discards_it() {
    let mut h = Harness::new();
    h.ui.window.invoke_show_options();
    let view = h.ui.window.global::<EditorOptionsView>();
    let mut draft = view.get_draft();
    draft.use_default = false;
    draft.executable = "relative-not-an-executable".into();
    view.invoke_edit(draft);
    h.ui.window.set_options_topic_index(0);
    h.ui.draw();
    ui::click_popup(&h.ui.window, "OK", AccessibleRole::Button);
    h.ui.draw();
    assert!(h.saves().is_empty());
    assert!(ui::has_popup(&h.ui.window));
    assert_eq!(h.ui.window.get_options_topic_index(), 4);
    assert!(
        !h.ui
            .window
            .global::<EditorOptionsView>()
            .get_error()
            .is_empty()
    );
    ui::click_popup(&h.ui.window, "Cancel", AccessibleRole::Button);
    assert!(!h.ui.window.global::<EditorOptionsView>().get_dirty());
    assert!(
        h.ui.window
            .global::<EditorOptionsView>()
            .get_draft()
            .use_default
    );
}

#[test]
fn ok_waits_for_color_validation_on_another_page_and_cancel_rejects_late_results() {
    let mut h = Harness::new();
    for cancel in [true, false] {
        h.ui.window.invoke_show_options();
        let colors = h.ui.window.global::<ColorOptionsView>();
        let mut draft = colors.get_draft();
        draft.foreground = "#123456".into();
        colors.invoke_edit(draft);
        h.ui.window.set_options_topic_index(0);
        h.ui.draw();
        ui::click_popup(&h.ui.window, "OK", AccessibleRole::Button);
        h.ui.draw();
        assert!(h.saves().is_empty());
        let request = h
            .sent
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::ValidateFilenameColors(request) => Some(request.clone()),
                _ => None,
            })
            .unwrap();
        if cancel {
            ui::click_popup(&h.ui.window, "Cancel", AccessibleRole::Button);
        }
        h.event(Event::FilenameColorsValidated {
            request,
            result: Ok(()),
        });
        if cancel {
            assert!(h.saves().is_empty());
            assert!(!ui::has_popup(&h.ui.window));
        } else {
            let saves = h.saves();
            assert_eq!(saves.len(), 1);
            assert_eq!(
                saves[0].1.filename_colors[0].style.foreground,
                Some(RgbColor(0x12, 0x34, 0x56))
            );
            h.event(Event::PreferencesSaved {
                revision: saves[0].0,
                result: Ok(midnight_core::FileComparatorSnapshot::default()),
            });
            assert!(!ui::has_popup(&h.ui.window));
        }
    }
}

#[test]
fn color_validation_error_selects_colors_page_and_preserves_draft() {
    let mut h = Harness::new();
    h.ui.window.invoke_show_options();
    let colors = h.ui.window.global::<ColorOptionsView>();
    let mut draft = colors.get_draft();
    draft.mode = 1;
    draft.expression = "[".into();
    colors.invoke_edit(draft);
    h.ui.draw();
    ui::click_popup(&h.ui.window, "OK", AccessibleRole::Button);
    let request = h
        .sent
        .borrow()
        .iter()
        .rev()
        .find_map(|command| match command {
            Command::ValidateFilenameColors(request) => Some(request.clone()),
            _ => None,
        })
        .unwrap();
    h.event(Event::FilenameColorsValidated {
        request,
        result: Err("Unclosed character class".into()),
    });
    assert_eq!(h.ui.window.get_options_topic_index(), 8);
    assert_eq!(
        h.ui.window
            .global::<ColorOptionsView>()
            .get_draft()
            .expression,
        "["
    );
    assert!(h.saves().is_empty());
    assert!(ui::has_popup(&h.ui.window));
}

/// The text of the line above the Options footer that says what Reset to defaults did.
fn reset_status(h: &Harness) -> Option<String> {
    use i_slint_core::accessibility::AccessibleStringProperty;
    ui::try_popup_item(&h.ui.window, "Options status", AccessibleRole::Text).map(|item| {
        item.accessible_string_property(AccessibleStringProperty::Value)
            .unwrap_or_default()
            .to_string()
    })
}

// U-D24: Reset to defaults has a visible effect on every click, also when every page
// already shows its defaults, and says what it did.
#[test]
fn reset_defaults_says_what_it_did_even_when_nothing_changes() {
    let mut h = Harness::new();
    h.ui.window.invoke_show_options();
    h.ui.draw();
    assert_eq!(reset_status(&h), None);
    let reset = ui::popup_item(&h.ui.window, "Reset to defaults", AccessibleRole::Button);
    let enabled = || {
        reset
            .accessible_string_property(
                i_slint_core::accessibility::AccessibleStringProperty::Enabled,
            )
            .is_some_and(|enabled| enabled == "true")
    };
    assert!(enabled());
    // Offered only when a click applies: not while a page's file chooser, an Export or
    // an Import is outstanding, when the presenter would ignore the click.
    for busy in [true, false] {
        h.ui.window
            .global::<midnight_ui::SettingsFileView>()
            .set_busy(busy);
        h.ui.draw();
        assert_eq!(
            enabled(),
            !busy,
            "Reset waits for a file chooser, an Export or an Import"
        );
    }
    ui::click_popup(&h.ui.window, "Reset to defaults", AccessibleRole::Button);
    h.ui.draw();
    assert_eq!(
        reset_status(&h).as_deref(),
        Some("Reset to defaults changed nothing: every page already showed its defaults.")
    );
    // A preview edit retires the report; Reset then applies the defaults and says so.
    h.ui.window.invoke_preferences_changed(1, 0, 0);
    h.ui.draw();
    assert_eq!(reset_status(&h), None);
    ui::click_popup(&h.ui.window, "Reset to defaults", AccessibleRole::Button);
    h.ui.draw();
    assert_eq!(h.ui.window.get_theme_index(), 0);
    assert_eq!(
        reset_status(&h).as_deref(),
        Some(
            "Reset to defaults: every page now shows its defaults. OK saves them; Cancel \
             keeps your settings."
        )
    );
    assert!(h.saves().is_empty(), "Reset is not a save action");
    ui::click_popup(&h.ui.window, "Cancel", AccessibleRole::Button);
    h.ui.draw();
    h.ui.window.invoke_show_options();
    h.ui.draw();
    assert_eq!(
        reset_status(&h),
        None,
        "a new session starts without a report"
    );
}
