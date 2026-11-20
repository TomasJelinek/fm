//! Options Export/Import configuration through the real footer buttons: the backend
//! answers are simulated, the Options transaction and its Cancel/OK are real.
use crate::support::ui;

use i_slint_core::items::AccessibleRole;
use midnight_core::*;
use midnight_ui::{Presenter, SettingsFileView, Theme, install, refresh};
use slint::ComponentHandle;
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
        theme: PreferenceTheme::Dark,
        bookmarks: vec![bookmark(3, "současný")],
        ..UserPreferences::default()
    }
}

fn imported() -> UserPreferences {
    UserPreferences {
        theme: PreferenceTheme::Nord,
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
                    preferences: current(),
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
            result: result.map(Box::new),
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
