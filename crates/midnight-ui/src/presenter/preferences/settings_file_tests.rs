use super::*;
use std::{cell::RefCell, io};

fn native(name: &str) -> PathBuf {
    std::env::temp_dir().join(name)
}

fn bookmark(id: u64, name: &str, slot: Option<u8>) -> Bookmark {
    Bookmark {
        id: BookmarkId(id),
        name: name.into(),
        path: native(name),
        archive: None,
        network: None,
        slot,
    }
}

fn geometry() -> WindowGeometry {
    WindowGeometry {
        normal: WindowRect {
            x: 40,
            y: 30,
            width: 1100,
            height: 700,
        },
        monitor: None,
        maximized: false,
        fullscreen: false,
    }
}

/// Every setting outside Options differs from the defaults, plus a few Options ones.
fn imported() -> UserPreferences {
    UserPreferences {
        theme: PreferenceTheme::Nord,
        density: UiDensity::Touch,
        quick_search_filter: true,
        command_history: vec![CommandHistoryEntry {
            command: "echo importováno 文档".into(),
            shell: ShellFlavor::PowerShell,
        }],
        file_list: FileListPreferences {
            template_history: vec!["$(FileName)$(TAB)$(FileSize)$(CRLF)".into()],
            destination: 1,
            path: Some(native("seznam.txt")),
            append: true,
        },
        checksum_algorithms: ChecksumAlgorithms::all(),
        window_geometry: Some(geometry()),
        panel_split_basis_points: 3100,
        viewer: ViewerPreferences {
            wrap: true,
            ..ViewerPreferences::default()
        },
        windows_shell: WindowsShell::Cmd,
        bookmarks: vec![bookmark(1, "imported 🧪", Some(4))],
        tabs: [
            PaneTabPreferences {
                paths: vec![native("levý"), native("文档")],
                active: 1,
                ..Default::default()
            },
            PaneTabPreferences {
                paths: vec![native("pravý")],
                active: 0,
                ..Default::default()
            },
        ],
        panel_view_modes: [PanelViewMode::Brief, PanelViewMode::Thumbnails],
        panel_filters: [
            PanelFilter {
                enabled: true,
                mode: FilterMode::Masks,
                expression: "*.rs".into(),
                case_sensitive: false,
            },
            PanelFilter::default(),
        ],
        panel_filter_history: vec![PanelFilter {
            enabled: true,
            mode: FilterMode::Masks,
            expression: "*.toml".into(),
            case_sensitive: false,
        }],
        ..UserPreferences::default()
    }
}

fn current() -> UserPreferences {
    UserPreferences {
        theme: PreferenceTheme::Dark,
        bookmarks: vec![bookmark(7, "current", Some(1))],
        ..UserPreferences::default()
    }
}

struct Harness {
    p: Presenter,
    sent: RefCell<Vec<Command>>,
}

/// Records commands; borrows only the log, so the presenter stays mutable.
fn sender(sent: &RefCell<Vec<Command>>) -> impl Fn(Command) -> bool + '_ {
    |command| {
        sent.borrow_mut().push(command);
        true
    }
}

impl Harness {
    fn new() -> Self {
        let mut p = Presenter::new("left".into(), "right".into());
        p.apply(Event::PreferencesLoaded(Ok(Box::new(
            AcceptedPreferences {
                preferences: current(),
                comparators: FileComparatorSnapshot::default(),
            },
        ))));
        p.begin_options();
        Self {
            p,
            sent: RefCell::default(),
        }
    }

    fn take(&self) -> Vec<Command> {
        std::mem::take(&mut self.sent.borrow_mut())
    }

    fn event(&mut self, event: Event) {
        let sent = &self.sent;
        self.p.apply_with_sender(event, &|command| {
            sent.borrow_mut().push(command);
            true
        });
    }

    /// Choose `path` in the Options footer chooser and return the backend request.
    fn choose(&mut self, export: bool, path: &Path) -> DesktopRequestId {
        let send = |command| {
            self.sent.borrow_mut().push(command);
            true
        };
        if export {
            self.p.begin_settings_export(PathPickerOwner::None, &send);
        } else {
            self.p.begin_settings_import(PathPickerOwner::None, &send);
        }
        let Some(Command::PickPath { request, input }) = self.take().pop() else {
            panic!("expected a native chooser");
        };
        let expected = if export {
            PathPickerKind::SettingsSave
        } else {
            PathPickerKind::SettingsOpen
        };
        assert_eq!(input.kind, expected);
        self.event(Event::PathPicked {
            request,
            result: Ok(Some(path.to_owned())),
        });
        // The first flush with a sender may also request unrelated metadata.
        let commands: Vec<_> = self
            .take()
            .into_iter()
            .filter(|command| {
                matches!(
                    command,
                    Command::ExportPreferences(_) | Command::ImportPreferences { .. }
                )
            })
            .collect();
        match commands.as_slice() {
            [Command::ExportPreferences(input)] if export => {
                assert_eq!(input.destination, path);
                input.request
            }
            [Command::ImportPreferences { request, source }] if !export => {
                assert_eq!(source, path);
                *request
            }
            other => panic!("unexpected commands: {}", other.len()),
        }
    }

    fn import(&mut self, preferences: UserPreferences) {
        let request = self.choose(false, &native("nastavení 🧪.json"));
        assert!(self.p.preferences.settings_file.importing());
        self.event(Event::PreferencesImported {
            request,
            result: Ok(Box::new(preferences)),
        });
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
}

#[test]
fn options_and_session_fields_together_restore_every_preference() {
    let source = imported();
    let mut restored = UserPreferences::default();
    options::restore_fields(&mut restored, &source);
    restore_session_fields(&mut restored, &source);
    assert_eq!(restored, source);
    // Missing placement or tabs keep what the session has.
    let mut kept = current();
    kept.window_geometry = Some(geometry());
    kept.tabs[1].paths = vec![native("kept")];
    let mut sparse = imported();
    sparse.window_geometry = None;
    sparse.tabs[1] = PaneTabPreferences::default();
    restore_session_fields(&mut kept, &sparse);
    assert_eq!(kept.window_geometry, Some(geometry()));
    assert_eq!(kept.tabs[1].paths, [native("kept")]);
    assert_eq!(kept.tabs[0], sparse.tabs[0]);
}

#[test]
fn import_previews_options_stages_the_session_and_cancel_restores_everything() {
    let mut h = Harness::new();
    h.import(imported());
    assert_eq!(h.p.preferences.values.theme, PreferenceTheme::Nord);
    assert!(h.p.preferences.values.quick_search_filter);
    assert_eq!(h.p.preferences.values.bookmarks, current().bookmarks);
    assert_eq!(h.p.preferences.values.tabs, current().tabs);
    assert!(
        h.p.preferences
            .settings_file
            .message
            .contains("Imported nastavení 🧪.json")
    );
    assert!(!h.p.preferences.settings_file.error);
    assert!(h.saves().is_empty(), "importing is a preview");
    h.p.cancel_options(&sender(&h.sent));
    assert_eq!(h.p.preferences.values, current());
    assert!(h.p.preferences.settings_file.staged.is_none());
    assert!(h.saves().is_empty());
    assert!(h.p.take_imported_window_geometry().is_none());
}

#[test]
fn ok_saves_the_whole_import_and_applies_session_state_only_after_success() {
    let mut h = Harness::new();
    h.import(imported());
    h.p.accept_options(&sender(&h.sent));
    let saves = h.saves();
    assert_eq!(saves.len(), 1);
    let (revision, saved) = saves[0].clone();
    let mut expected = imported();
    // Fresh identities above every bookmark the session has seen.
    expected.bookmarks[0].id = BookmarkId(8);
    assert_eq!(saved, expected);
    // Nothing outside Options changes before the save succeeds.
    assert_eq!(h.p.preferences.values.bookmarks, current().bookmarks);
    assert!(!h.p.imported_window_geometry_pending());
    h.event(Event::PreferencesSaved {
        revision,
        result: Ok(FileComparatorSnapshot::default()),
    });
    assert!(!h.p.preferences.options.open);
    assert_eq!(h.p.preferences.values.bookmarks, expected.bookmarks);
    assert_eq!(h.p.preferences.values.viewer, expected.viewer);
    assert_eq!(h.p.preferences.values.panel_filters, expected.panel_filters);
    assert_eq!(h.p.take_imported_window_geometry(), Some(geometry()));
    assert!(h.p.take_imported_window_geometry().is_none());
    assert_eq!(h.p.preferences.values.tabs, expected.tabs);
    let tabs = h.p.tabs(PaneId::Left);
    assert_eq!(tabs.len(), 2, "the imported tabs were restored");
}

#[test]
fn a_failed_commit_keeps_the_staged_session_and_cancel_still_discards_it() {
    let mut h = Harness::new();
    h.import(imported());
    h.p.accept_options(&sender(&h.sent));
    let (revision, _) = h.saves()[0].clone();
    h.event(Event::PreferencesSaved {
        revision,
        result: Err(FsError::from_io(
            "save",
            Path::new("settings"),
            io::Error::from(io::ErrorKind::PermissionDenied),
        )
        .into()),
    });
    assert!(h.p.preferences.options.open);
    assert!(h.p.preferences.settings_file.staged.is_some());
    assert_eq!(h.p.preferences.values.bookmarks, current().bookmarks);
    h.p.cancel_options(&sender(&h.sent));
    assert_eq!(h.p.preferences.values, current());
    assert!(!h.p.imported_window_geometry_pending());
}

#[test]
fn refused_and_late_imports_change_nothing() {
    let mut h = Harness::new();
    let request = h.choose(false, &native("broken.json"));
    h.event(Event::PreferencesImported {
        request,
        result: Err(FsError::from_io(
            "import settings",
            &native("broken.json"),
            io::Error::new(
                io::ErrorKind::InvalidData,
                "The settings are invalid: unknown field `x`",
            ),
        )),
    });
    let state = &h.p.preferences.settings_file;
    assert!(state.error);
    assert!(
        state.message.contains("unknown field `x`."),
        "{}",
        state.message
    );
    assert!(state.message.ends_with("Your settings are unchanged."));
    assert_eq!(h.p.preferences.values, current());
    // A result arriving after Cancel cannot stage into a later session.
    let request = h.choose(false, &native("late.json"));
    h.p.cancel_options(&sender(&h.sent));
    h.p.begin_options();
    h.event(Event::PreferencesImported {
        request,
        result: Ok(Box::new(imported())),
    });
    assert_eq!(h.p.preferences.values, current());
    assert!(h.p.preferences.settings_file.staged.is_none());
    assert!(!h.p.preferences.settings_file.busy());
}

#[test]
fn export_sends_what_ok_would_save_and_replaces_only_with_consent() {
    let mut h = Harness::new();
    h.p.change_preferences(PreferenceTheme::Nord.index(), 0, 0, &sender(&h.sent));
    assert!(h.saves().is_empty());
    let path = native("export 文档.json");
    let request = h.choose(true, &path);
    assert!(h.p.preferences.settings_file.busy());
    h.event(Event::PreferencesExported {
        request,
        result: Err(FsError::from_io(
            "export settings",
            &path,
            io::Error::from(io::ErrorKind::AlreadyExists),
        )),
    });
    assert!(h.p.preferences.settings_file.replace_pending());
    assert!(
        h.p.preferences
            .settings_file
            .message
            .contains("already exists")
    );
    h.p.replace_settings_export(&sender(&h.sent));
    let Some(Command::ExportPreferences(input)) = h
        .take()
        .into_iter()
        .find(|command| matches!(command, Command::ExportPreferences(_)))
    else {
        panic!("Replace must export again");
    };
    assert!(input.overwrite);
    assert_eq!(input.preferences.theme, PreferenceTheme::Nord);
    assert_eq!(input.preferences.bookmarks, current().bookmarks);
    let receipt = PreferencesExportReceipt {
        path: path.clone(),
        bytes: 10,
        recovery: vec![native("export 文档.json.me-0123456789abcdef")],
    };
    h.event(Event::PreferencesExported {
        request: input.request,
        result: Ok(receipt),
    });
    let state = &h.p.preferences.settings_file;
    assert!(!state.error && !state.replace_pending() && !state.busy());
    assert!(
        state.message.starts_with("Exported settings to"),
        "{}",
        state.message
    );
    assert!(state.message.contains("The replaced file was kept as"));
    // Keep answers the prompt without writing anything.
    let request = h.choose(true, &path);
    h.event(Event::PreferencesExported {
        request,
        result: Err(FsError::from_io(
            "export settings",
            &path,
            io::Error::from(io::ErrorKind::AlreadyExists),
        )),
    });
    h.p.keep_settings_export();
    assert!(!h.p.preferences.settings_file.replace_pending());
    assert!(
        !h.take()
            .iter()
            .any(|command| matches!(command, Command::ExportPreferences(_)))
    );
}

#[test]
fn an_export_finishing_after_options_closed_reports_outside_the_dialog() {
    let mut h = Harness::new();
    let request = h.choose(true, &native("closed.json"));
    h.p.cancel_options(&sender(&h.sent));
    h.event(Event::PreferencesExported {
        request,
        result: Ok(PreferencesExportReceipt {
            path: native("closed.json"),
            bytes: 1,
            recovery: Vec::new(),
        }),
    });
    assert!(h.p.desktop.status.starts_with("Exported settings to"));
    assert!(h.p.preferences.settings_file.message.is_empty());
}

#[test]
fn ok_and_reset_wait_for_a_pending_import_and_reset_drops_a_staged_one() {
    let mut h = Harness::new();
    let request = h.choose(false, &native("pending.json"));
    h.p.accept_options(&sender(&h.sent));
    h.p.reset_options(&sender(&h.sent));
    assert!(h.saves().is_empty());
    assert!(h.p.preferences.options.open);
    h.event(Event::PreferencesImported {
        request,
        result: Ok(Box::new(imported())),
    });
    assert!(h.p.preferences.settings_file.staged.is_some());
    h.p.reset_options(&sender(&h.sent));
    assert!(h.p.preferences.settings_file.staged.is_none());
    assert_eq!(h.p.preferences.values.theme, PreferenceTheme::default());
    assert_eq!(h.p.preferences.values.bookmarks, current().bookmarks);
}
