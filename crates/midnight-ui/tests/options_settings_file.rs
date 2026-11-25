//! Options Export/Import configuration through the real footer buttons: the backend
//! answers are simulated, the Options transaction and its Cancel/OK are real.
use crate::support::ui;

use i_slint_core::{
    accessibility::{AccessibilityAction, AccessibleStringProperty},
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::AccessibleRole,
    window::WindowInner,
};
use midnight_core::*;
use midnight_ui::{Presenter, SettingsFileView, Theme, install, refresh};
use slint::{ComponentHandle, platform::Key};
use std::{
    cell::RefCell,
    io,
    path::{Path, PathBuf},
    rc::Rc,
};

struct Harness {
    ui: ui::TestUi,
    presenter: Rc<RefCell<Presenter>>,
    sent: Rc<RefCell<Vec<Command>>>,
}

fn native(name: &str) -> PathBuf {
    std::env::temp_dir().join(name)
}

fn bookmark(id: u64, name: &str) -> Bookmark {
    Bookmark {
        id: BookmarkId(id),
        name: name.into(),
        path: native(name),
        archive: None,
        network: None,
        slot: Some(2),
    }
}

fn geometry() -> WindowGeometry {
    WindowGeometry {
        normal: WindowRect {
            x: 30,
            y: 20,
            width: 1000,
            height: 680,
        },
        monitor: None,
        maximized: false,
        fullscreen: false,
    }
}

fn current() -> UserPreferences {
    UserPreferences {
        dark_theme: PreferenceTheme::Dark,
        bookmarks: vec![bookmark(3, "současný")],
        ..UserPreferences::default()
    }
}

fn imported() -> UserPreferences {
    UserPreferences {
        dark_theme: PreferenceTheme::Nord,
        bookmarks: vec![bookmark(1, "importovaný 文档")],
        tabs: [
            PaneTabPreferences {
                paths: vec![native("levý 🧪"), native("druhý")],
                active: 0,
                ..Default::default()
            },
            PaneTabPreferences::default(),
        ],
        window_geometry: Some(geometry()),
        ..UserPreferences::default()
    }
}

impl Harness {
    fn new() -> Self {
        Self::with(current())
    }

    fn with(preferences: UserPreferences) -> Self {
        let ui = ui::TestUi::new();
        // Absolute like real panes: OK persists the pane the import leaves unchanged.
        let presenter = Rc::new(RefCell::new(Presenter::new(
            native("left"),
            native("right"),
        )));
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
                AcceptedPreferences {
                    preferences,
                    comparators: FileComparatorSnapshot::default(),
                },
            ))));
        refresh(&ui.window, &presenter.borrow());
        let mut harness = Self {
            ui,
            presenter,
            sent,
        };
        harness.ui.window.invoke_show_options();
        harness.ui.draw();
        harness.sent.borrow_mut().clear();
        harness
    }

    fn take(&self) -> Vec<Command> {
        std::mem::take(&mut self.sent.borrow_mut())
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

    fn click(&mut self, label: &str) {
        ui::click_popup(&self.ui.window, label, AccessibleRole::Button);
        self.ui.draw();
    }

    /// Click a footer button, answer its native chooser and return the backend command.
    fn choose(&mut self, label: &str, kind: PathPickerKind, path: &Path) -> Command {
        self.click(label);
        let Some(Command::PickPath { request, input }) = self
            .take()
            .into_iter()
            .find(|command| matches!(command, Command::PickPath { .. }))
        else {
            panic!("{label} must open a native chooser");
        };
        assert_eq!(input.kind, kind);
        self.event(Event::PathPicked {
            request,
            result: Ok(Some(path.to_owned())),
        });
        let mut transfers: Vec<_> = self
            .take()
            .into_iter()
            .filter(|command| {
                matches!(
                    command,
                    Command::ExportPreferences(_) | Command::ImportPreferences { .. }
                )
            })
            .collect();
        assert_eq!(transfers.len(), 1, "one backend request per choice");
        transfers.remove(0)
    }

    fn import(&mut self, result: Result<UserPreferences, FsError>) {
        let path = native("nastavení 文档.json");
        let Command::ImportPreferences { request, source } =
            self.choose("Import configuration…", PathPickerKind::SettingsOpen, &path)
        else {
            panic!("Import must read the chosen file on the backend");
        };
        assert_eq!(source, path);
        assert!(self.view().get_importing());
        self.event(Event::PreferencesImported {
            request,
            result: result.map(|preferences| {
                Box::new(ImportedPreferences {
                    preferences,
                    skipped_tabs: 0,
                    removed_bindings: 0,
                })
            }),
        });
    }

    fn view(&self) -> SettingsFileView<'_> {
        self.ui.window.global::<SettingsFileView>()
    }

    fn scheme(&self) -> i32 {
        self.ui.window.global::<Theme>().get_scheme()
    }

    fn saves(commands: &[Command]) -> Vec<(PreferencesRevision, UserPreferences)> {
        commands
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
}

#[test]
fn export_button_sends_the_shown_settings_and_replaces_only_after_consent() {
    let mut h = Harness::new();
    let path = native("export 文档.json");
    let Command::ExportPreferences(request) =
        h.choose("Export configuration…", PathPickerKind::SettingsSave, &path)
    else {
        panic!("Export must write on the backend");
    };
    assert_eq!(request.destination, path);
    assert!(!request.overwrite);
    assert_eq!(request.preferences, current());
    assert!(h.view().get_busy());
    h.event(Event::PreferencesExported {
        request: request.request,
        result: Err(FsError::from_io(
            "export settings",
            &path,
            io::Error::from(io::ErrorKind::AlreadyExists),
        )),
    });
    assert!(h.view().get_replace_pending());
    assert!(h.view().get_message().contains("already exists"));
    h.click("Replace");
    let Some(Command::ExportPreferences(replace)) = h
        .take()
        .into_iter()
        .find(|command| matches!(command, Command::ExportPreferences(_)))
    else {
        panic!("Replace must export again");
    };
    assert!(replace.overwrite && replace.destination == path);
    h.event(Event::PreferencesExported {
        request: replace.request,
        result: Ok(PreferencesExportReceipt {
            path: path.clone(),
            bytes: 1024,
            recovery: Vec::new(),
        }),
    });
    let status = ui::popup_item(&h.ui.window, "Settings file status", AccessibleRole::Text);
    assert!(
        ui::text_values(status)
            .iter()
            .any(|text| text.starts_with("Exported settings to")),
    );
    assert!(!h.view().get_busy() && !h.view().get_replace_pending());
    assert!(
        Harness::saves(&h.take()).is_empty(),
        "exporting saves nothing"
    );
    assert!(ui::has_popup(&h.ui.window), "Options stays open");
}

#[test]
fn import_button_previews_and_cancel_restores_the_previous_settings() {
    let mut h = Harness::new();
    let dark = PreferenceTheme::Dark.index();
    assert_eq!(h.scheme(), dark);
    h.import(Ok(imported()));
    assert_eq!(
        h.scheme(),
        PreferenceTheme::Nord.index(),
        "imported theme previews"
    );
    assert_eq!(h.presenter.borrow().bookmarks(), current().bookmarks);
    assert!(
        h.view()
            .get_message()
            .starts_with("Imported nastavení 文档.json.")
    );
    assert!(
        Harness::saves(&h.take()).is_empty(),
        "importing saves nothing"
    );
    h.click("Cancel");
    assert!(!ui::has_popup(&h.ui.window));
    assert_eq!(h.scheme(), dark, "Cancel restores the previous theme");
    let presenter = h.presenter.borrow();
    assert_eq!(presenter.bookmarks(), current().bookmarks);
    assert_eq!(presenter.tabs(PaneId::Left).len(), 1);
    assert!(!presenter.imported_window_geometry_pending());
    drop(presenter);
    assert!(Harness::saves(&h.take()).is_empty());
    // Reopening starts from the accepted settings without the discarded import.
    h.ui.window.invoke_show_options();
    h.ui.draw();
    assert_eq!(h.view().get_message(), "");
    assert_eq!(h.scheme(), dark);
}

#[test]
fn import_then_ok_saves_the_whole_file_and_applies_tabs_hot_paths_and_placement() {
    let mut h = Harness::new();
    h.import(Ok(imported()));
    h.click("OK");
    let saves = Harness::saves(&h.take());
    assert_eq!(saves.len(), 1);
    let mut expected = imported();
    // Imported hot paths get identities the session has never shown.
    expected.bookmarks[0].id = BookmarkId(4);
    assert_eq!(saves[0].1, expected);
    assert_eq!(h.presenter.borrow().bookmarks(), current().bookmarks);
    h.event(Event::PreferencesSaved {
        revision: saves[0].0,
        result: Ok(FileComparatorSnapshot::default()),
    });
    assert!(!ui::has_popup(&h.ui.window));
    assert_eq!(h.scheme(), PreferenceTheme::Nord.index());
    let mut presenter = h.presenter.borrow_mut();
    assert_eq!(presenter.bookmarks(), expected.bookmarks);
    assert_eq!(presenter.tabs(PaneId::Left).len(), 2);
    assert_eq!(
        presenter.tabs(PaneId::Right).len(),
        1,
        "no saved tabs keep the pane"
    );
    assert_eq!(presenter.take_imported_window_geometry(), Some(geometry()));
    drop(presenter);
    // Restoring the imported tabs persists both panes again: only valid sessions.
    let saves = Harness::saves(&h.take());
    assert!(saves.iter().all(|(_, saved)| saved.validate().is_ok()));
}

#[test]
fn a_refused_import_shows_its_error_and_changes_nothing() {
    let mut h = Harness::new();
    h.import(Err(FsError::from_io(
        "import settings",
        &native("nastavení 文档.json"),
        io::Error::new(
            io::ErrorKind::InvalidData,
            "The file uses settings version 99 from a newer Midnight Explorer.",
        ),
    )));
    let error = ui::popup_item(&h.ui.window, "Settings file error", AccessibleRole::Text);
    assert!(
        ui::text_values(error)
            .iter()
            .any(|text| text.contains("settings version 99") && text.contains("unchanged")),
    );
    assert!(h.view().get_error());
    assert_eq!(h.scheme(), PreferenceTheme::Dark.index());
    assert_eq!(h.presenter.borrow().bookmarks(), current().bookmarks);
    h.click("Cancel");
    assert!(Harness::saves(&h.take()).is_empty());
}

/// Window bounds of an Options control: left, top, right, bottom.
fn bounds(item: &ItemRc) -> (f32, f32, f32, f32) {
    let geometry = item.geometry();
    let origin = item.map_to_native_window(geometry.origin);
    (
        origin.x,
        origin.y,
        origin.x + geometry.width(),
        origin.y + geometry.height(),
    )
}

/// The label of the focused control, or of its nearest labelled parent.
fn focused(h: &Harness) -> Option<String> {
    let inner = WindowInner::from_pub(h.ui.window.window());
    let mut current = inner.focus_item.borrow().upgrade();
    while let Some(item) = current {
        if let Some(label) = item.accessible_string_property(AccessibleStringProperty::Label)
            && !label.is_empty()
        {
            return Some(label.to_string());
        }
        current = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    None
}

#[test]
fn export_and_import_leave_a_footer_without_room_and_ok_stays_in_view() {
    let mut h = Harness::with(UserPreferences {
        desktop_font_size: 32,
        touch_font_size: 32,
        ..current()
    });
    let button = |h: &Harness, label: &str| {
        bounds(&ui::popup_item(&h.ui.window, label, AccessibleRole::Button))
    };
    // A large window has room for their icons beside Reset, OK and Cancel.
    let ok = button(&h, "OK");
    assert_eq!(button(&h, "Export configuration…").1, ok.1, "in the footer");
    ui::key(&h.ui.window, Key::Backtab);
    h.ui.draw();
    assert_eq!(focused(&h).as_deref(), Some("Import configuration…"));
    // The smallest window has not: they move to the General page, and the focus
    // they had moves to OK, which stays in view beside Cancel.
    h.ui.resize(320, 200);
    h.ui.draw();
    let (ok, cancel) = (button(&h, "OK"), button(&h, "Cancel"));
    assert!(
        ok.0 >= 0.0 && ok.2 <= cancel.0 && ok.3 <= 200.0,
        "OK {ok:?} beside Cancel {cancel:?}"
    );
    for label in ["Export configuration…", "Import configuration…"] {
        assert_ne!(button(&h, label).1, ok.1, "{label} left the footer");
    }
    assert_eq!(focused(&h).as_deref(), Some("OK"));
    ui::key(&h.ui.window, Key::Escape);
    h.ui.draw();
    assert!(!h.ui.window.get_options_open(), "Escape closes Options");
    // Reopened at this size, the General page's Export still opens the chooser.
    h.ui.window.invoke_show_options();
    h.ui.draw();
    h.take();
    ui::popup_item(
        &h.ui.window,
        "Export configuration…",
        AccessibleRole::Button,
    )
    .accessible_action(&AccessibilityAction::Default);
    h.ui.draw();
    assert!(h.take().iter().any(|command| matches!(
        command,
        Command::PickPath { input, .. } if input.kind == PathPickerKind::SettingsSave
    )));
}

/// An absolute path written by the other operating system.
fn other(name: &str) -> String {
    match PathSystem::CURRENT.other() {
        PathSystem::Unix => format!("/home/tom/{name}"),
        PathSystem::Windows => format!("C:\\Users\\tom\\{name}"),
    }
}

fn kept(kind: ForeignSettingKind, location: &str) -> ForeignSetting {
    ForeignSetting {
        kind,
        system: PathSystem::CURRENT.other(),
        location: other(location),
        stored: "{}".into(),
    }
}

#[test]
fn settings_kept_from_another_system_are_listed_inactive_and_removed_as_an_options_edit() {
    use slint::Model;
    let preferences = UserPreferences {
        foreign_settings: vec![
            kept(ForeignSettingKind::Editor, "code"),
            kept(
                ForeignSettingKind::UserCommand {
                    position: 0,
                    id: UserCommandId::new("custom-1").unwrap(),
                    name: "Sestavit".into(),
                },
                "make",
            ),
        ]
        .into(),
        foreign_hot_paths: vec![ForeignHotPath {
            id: BookmarkId(9),
            slot: Some(4),
            system: PathSystem::CURRENT.other(),
            location: other("projekty 文档"),
            stored: "{}".into(),
        }],
        ..current()
    };
    let mut h = Harness::with(preferences);
    // An ordinary window: the kept list then lies below the General page's fold, which
    // `the_kept_list_scrolls_fully_into_view_at_the_end_of_the_general_page` scrolls to.
    h.ui.resize(940, 660);
    h.ui.window.set_options_topic_index(0);
    h.ui.draw();
    let system = PathSystem::CURRENT.other().name();
    assert_eq!(
        h.view().get_kept_title().as_str(),
        format!("Kept from {system}")
    );
    assert_eq!(h.view().get_kept_rows().row_count(), 2);
    let editor = ui::popup_item(&h.ui.window, "Editor", AccessibleRole::Text);
    assert!(
        ui::text_values(editor)
            .iter()
            .any(|text| text.contains(&other("code")))
    );
    // The kept list ends the long General page: its buttons' accessible actions act
    // without scrolling.
    let remove = |h: &mut Harness, label: &str| {
        ui::popup_item(&h.ui.window, label, AccessibleRole::Button)
            .accessible_action(&AccessibilityAction::Default);
        h.ui.draw();
    };
    remove(&mut h, "Remove Editor");
    assert_eq!(h.view().get_kept_rows().row_count(), 1);
    assert!(Harness::saves(&h.take()).is_empty(), "a preview until OK");
    h.click("Cancel");
    h.ui.window.invoke_show_options();
    h.ui.draw();
    assert_eq!(h.view().get_kept_rows().row_count(), 2, "Cancel keeps it");
    // The Hot Paths manager lists the inactive hot path with the reason.
    let rows = h.ui.window.get_bookmarks();
    let row = (0..rows.row_count())
        .filter_map(|index| rows.row_data(index))
        .find(|row| row.identity == "9")
        .unwrap();
    assert_eq!(row.slot, 4);
    assert_eq!(
        row.note.as_str(),
        format!(
            "{system} path, not available on {}",
            PathSystem::CURRENT.name()
        )
    );
    remove(&mut h, "Remove Custom command “Sestavit”");
    h.click("OK");
    let saves = Harness::saves(&h.take());
    let (_, saved) = saves.last().unwrap();
    assert_eq!(
        *saved.foreign_settings,
        [kept(ForeignSettingKind::Editor, "code")]
    );
    assert_eq!(saved.foreign_hot_paths.len(), 1);
}

/// Window bounds of a footer button of the open Options dialog.
fn footer_button(h: &Harness, label: &str) -> (f32, f32, f32, f32) {
    bounds(&ui::popup_item(&h.ui.window, label, AccessibleRole::Button))
}

#[test]
fn ok_and_cancel_share_one_row_while_the_footer_actions_fit_beside_cancel() {
    let mut h = Harness::new();
    // Narrower than the footer's former fixed 440 px stacking width, with room for
    // Reset, OK and Cancel in one row.
    h.ui.resize(420, 600);
    let (ok, cancel) = (footer_button(&h, "OK"), footer_button(&h, "Cancel"));
    assert_eq!(
        ok.1, cancel.1,
        "OK {ok:?} and Cancel {cancel:?} share one row"
    );
    assert!(ok.2 <= cancel.0, "OK {ok:?} sits left of Cancel {cancel:?}");
}

#[test]
fn ok_and_cancel_stay_whole_and_apart_with_a_large_font_at_the_narrowest_width() {
    // Whether or not Cancel needs a row of its own, OK and Cancel stay whole, in view
    // and apart.
    let mut h = Harness::with(UserPreferences {
        desktop_font_size: 32,
        touch_font_size: 32,
        ..current()
    });
    h.ui.resize(320, 600);
    let (ok, cancel) = (footer_button(&h, "OK"), footer_button(&h, "Cancel"));
    for (label, (left, top, right, bottom)) in [("OK", ok), ("Cancel", cancel)] {
        assert!(
            left >= 0.0 && top >= 0.0 && right <= 320.0 && bottom <= 600.0,
            "{label} {:?} in view",
            (left, top, right, bottom)
        );
    }
    assert!(
        ok.2 <= cancel.0 || ok.3 <= cancel.1,
        "OK {ok:?} beside or above Cancel {cancel:?}, never over it"
    );
}

#[test]
fn the_kept_list_scrolls_fully_into_view_at_the_end_of_the_general_page() {
    use i_slint_core::items::Flickable;
    let preferences = UserPreferences {
        foreign_settings: vec![
            kept(ForeignSettingKind::Editor, "code"),
            kept(
                ForeignSettingKind::UserCommand {
                    position: 0,
                    id: UserCommandId::new("custom-1").unwrap(),
                    name: "Sestavit".into(),
                },
                "make",
            ),
        ]
        .into(),
        ..current()
    };
    let mut h = Harness::with(preferences);
    // A short window (the polish gate's smallest), shorter than the General page with
    // its kept list on every platform's fonts.
    h.ui.resize(940, 420);
    h.ui.window.set_options_topic_index(0);
    h.ui.draw();
    let label = "Remove Custom command “Sestavit”";
    // The last kept row's Remove button, the page's scroll view and its bounds.
    let page = |h: &Harness| {
        let item = ui::popup_item(&h.ui.window, label, AccessibleRole::Button);
        let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
        loop {
            let ancestor = parent.expect("the General page scrolls");
            if ancestor.downcast::<Flickable>().is_some() {
                return (bounds(&item), bounds(&ancestor), ancestor);
            }
            parent = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
        }
    };
    let (_, view, _) = page(&h);
    // Scroll as far as the page goes.
    for _ in 0..12 {
        h.ui.window
            .window()
            .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                position: slint::LogicalPosition::new(
                    (view.0 + view.2) / 2.0,
                    (view.1 + view.3) / 2.0,
                ),
                delta_x: 0.0,
                delta_y: -400.0,
            });
        h.ui.draw();
    }
    let (remove, view, flickable) = page(&h);
    let offset = flickable
        .downcast::<Flickable>()
        .unwrap()
        .as_pin_ref()
        .content_y()
        .get();
    assert!(offset < 0.0, "the General page scrolled: {offset}");
    assert!(
        remove.1 >= view.1 - 1.0 && remove.3 <= view.3 + 1.0,
        "{label} {remove:?} is wholly inside the page {view:?}"
    );
}
