use super::*;
use std::{cell::RefCell, io};

fn native(name: &str) -> PathBuf {
    std::env::temp_dir().join(name)
}

/// A validated file without entries of another system.
fn summary(preferences: UserPreferences) -> Box<ImportedPreferences> {
    Box::new(ImportedPreferences {
        preferences,
        skipped_tabs: 0,
        removed_bindings: 0,
    })
}

/// An absolute path written by the other operating system.
fn other(name: &str) -> String {
    match PathSystem::CURRENT.other() {
        PathSystem::Unix => format!("/home/tom/{name}"),
        PathSystem::Windows => format!("C:\\Users\\tom\\{name}"),
    }
}

fn inactive_hot_path(id: u64, slot: Option<u8>) -> ForeignHotPath {
    ForeignHotPath {
        id: BookmarkId(id),
        slot,
        system: PathSystem::CURRENT.other(),
        location: other("projekty 文档"),
        stored: "{}".into(),
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
        dark_theme: PreferenceTheme::Nord,
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
        dark_theme: PreferenceTheme::Dark,
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
            result: Ok(summary(preferences)),
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
    assert_eq!(h.p.preferences.values.dark_theme, PreferenceTheme::Nord);
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
        result: Ok(summary(imported())),
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
    assert_eq!(input.preferences.dark_theme, PreferenceTheme::Nord);
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
        result: Ok(summary(imported())),
    });
    assert!(h.p.preferences.settings_file.staged.is_some());
    h.p.reset_options(&sender(&h.sent));
    assert!(h.p.preferences.settings_file.staged.is_none());
    assert_eq!(
        h.p.preferences.values.dark_theme,
        PreferenceTheme::DEFAULT_DARK
    );
    assert_eq!(h.p.preferences.values.bookmarks, current().bookmarks);
}

/// U-D24: Reset waits for an Export too, as the footer shows it disabled: a click the
/// footer offers always applies.
#[test]
fn reset_waits_for_a_pending_export() {
    let mut h = Harness::new();
    let request = h.choose(true, &native("saved.json"));
    h.p.reset_options(&sender(&h.sent));
    assert_eq!(h.p.preferences.values.dark_theme, PreferenceTheme::Dark);
    assert!(h.p.preferences.options.reset_status.is_empty());
    h.event(Event::PreferencesExported {
        request,
        result: Ok(PreferencesExportReceipt {
            path: native("saved.json"),
            bytes: 1,
            recovery: Vec::new(),
        }),
    });
    h.p.reset_options(&sender(&h.sent));
    assert_eq!(
        h.p.preferences.values.dark_theme,
        PreferenceTheme::DEFAULT_DARK
    );
    assert!(
        h.p.preferences
            .options
            .reset_status
            .starts_with("Reset to defaults: every page now shows its defaults.")
    );
}

fn with_kept(foreign_hot_paths: Vec<ForeignHotPath>, foreign: Vec<ForeignSetting>) -> Presenter {
    let mut p = Presenter::new(native("left"), native("right"));
    p.apply(Event::PreferencesLoaded(Ok(Box::new(
        AcceptedPreferences {
            preferences: UserPreferences {
                foreign_hot_paths,
                foreign_settings: foreign.into(),
                ..current()
            },
            comparators: FileComparatorSnapshot::default(),
        },
    ))));
    p
}

#[test]
fn inactive_hot_paths_explain_themselves_share_slots_and_can_be_removed() {
    let mut p = with_kept(vec![inactive_hot_path(8, Some(2))], Vec::new());
    let sent = RefCell::new(Vec::new());
    assert!(p.jump_bookmark(PaneId::Left, 2, &sender(&sent)));
    let (title, body) = p.desktop.notice.clone().unwrap();
    assert_eq!(title, "Hot Path not available");
    assert!(body.contains(&other("projekty 文档")), "{body}");
    assert!(body.contains(PathSystem::CURRENT.other().name()));
    assert!(p.open_bookmark(PaneId::Left, BookmarkId(8), &sender(&sent)));
    assert!(sent.borrow().is_empty(), "nothing navigates or saves");
    // Numbered slots are shared: taking slot 2 hands slot 1 to the inactive path.
    assert!(p.set_bookmark_slot(BookmarkId(7), Some(2), &sender(&sent)));
    assert_eq!(p.bookmarks()[0].slot, Some(2));
    assert_eq!(p.foreign_hot_paths()[0].slot, Some(1));
    // A new hot path gets a fresh identity and the first slot nobody holds.
    assert!(p.add_bookmark_location(bookmark(1, "nový", None), &sender(&sent)));
    assert_eq!(p.bookmarks()[1].id, BookmarkId(9));
    assert_eq!(p.bookmarks()[1].slot, Some(3));
    assert!(p.remove_bookmark(BookmarkId(8), &sender(&sent)));
    assert!(p.foreign_hot_paths().is_empty());
    let saved = sent
        .borrow()
        .iter()
        .rev()
        .find_map(|command| match command {
            Command::SavePreferences { preferences, .. } => Some(preferences.clone()),
            _ => None,
        });
    assert!(saved.unwrap().foreign_hot_paths.is_empty());
}

#[test]
fn an_import_reports_what_the_other_system_keeps_and_leaves_out() {
    let mut h = Harness::new();
    let request = h.choose(false, &native("linux.json"));
    let preferences = UserPreferences {
        foreign_hot_paths: vec![inactive_hot_path(1, Some(5))],
        foreign_settings: vec![
            kept(ForeignSettingKind::Editor, "code"),
            kept(
                ForeignSettingKind::UserCommand {
                    position: 0,
                    id: UserCommandId::new("custom-1").unwrap(),
                    name: "Build".into(),
                },
                "make",
            ),
        ]
        .into(),
        ..imported()
    };
    h.event(Event::PreferencesImported {
        request,
        result: Ok(Box::new(ImportedPreferences {
            preferences,
            skipped_tabs: 2,
            removed_bindings: 1,
        })),
    });
    let system = PathSystem::CURRENT.other().name();
    let message = h.p.preferences.settings_file.message.clone();
    for expected in [
        format!(
            "Kept inactive because they use {system} paths: 1 hot path, 1 custom command and the editor."
        ),
        format!("Not opened: 2 tabs at {system} folders."),
        "Kept inactive with those custom commands: 1 key binding.".into(),
    ] {
        assert!(message.contains(&expected), "{message}");
    }
    // Kept settings preview with the Options pages; the inactive hot path waits for OK.
    assert_eq!(h.p.preferences.values.foreign_settings.len(), 2);
    assert!(h.p.foreign_hot_paths().is_empty());
    h.p.accept_options(&sender(&h.sent));
    let (revision, saved) = h.saves()[0].clone();
    assert_eq!(saved.foreign_settings.len(), 2);
    // Fresh identity after the native hot paths of the file and of the session.
    assert_eq!(saved.foreign_hot_paths[0].id, BookmarkId(9));
    assert_eq!(saved.bookmarks[0].id, BookmarkId(8));
    h.event(Event::PreferencesSaved {
        revision,
        result: Ok(FileComparatorSnapshot::default()),
    });
    assert_eq!(h.p.foreign_hot_paths(), saved.foreign_hot_paths);
}

#[test]
fn removing_a_kept_setting_is_an_options_preview_that_cancel_restores() {
    let editor = kept(ForeignSettingKind::Editor, "code");
    let command = kept(
        ForeignSettingKind::UserCommand {
            position: 1,
            id: UserCommandId::new("custom-1").unwrap(),
            name: "Build".into(),
        },
        "make",
    );
    let mut p = with_kept(Vec::new(), vec![editor.clone(), command.clone()]);
    let sent = RefCell::new(Vec::new());
    p.begin_options();
    let shown = crate::shown_text(&editor.location).to_string();
    p.remove_foreign_setting(1, &shown, &sender(&sent));
    assert_eq!(p.preferences.values.foreign_settings.len(), 2, "stale row");
    p.remove_foreign_setting(0, &shown, &sender(&sent));
    assert_eq!(
        *p.preferences.values.foreign_settings,
        std::slice::from_ref(&command)
    );
    assert!(sent.borrow().is_empty(), "a preview until OK");
    p.cancel_options(&sender(&sent));
    assert_eq!(*p.preferences.values.foreign_settings, [editor, command]);
    // Reset to defaults keeps them, like custom commands.
    p.begin_options();
    p.reset_options(&sender(&sent));
    assert_eq!(p.preferences.values.foreign_settings.len(), 2);
}

#[test]
fn a_chosen_editor_or_output_file_replaces_the_kept_one() {
    let editor = kept(ForeignSettingKind::Editor, "code");
    let output = kept(ForeignSettingKind::FileListOutput, "seznam.txt");
    let mut p = with_kept(Vec::new(), vec![editor.clone(), output.clone()]);
    let sent = RefCell::new(Vec::new());
    p.begin_options();
    let exe = std::env::current_exe().unwrap();
    p.edit_editor_draft(exe.to_string_lossy().into_owned(), false);
    p.save_editor_draft(&sender(&sent));
    assert_eq!(p.preferences.values.editor.executable, Some(exe));
    assert_eq!(
        *p.preferences.values.foreign_settings,
        std::slice::from_ref(&output)
    );
    // The editor is an Options edit: Cancel brings the kept one back.
    p.cancel_options(&sender(&sent));
    assert_eq!(p.preferences.values.editor.executable, None);
    assert_eq!(
        *p.preferences.values.foreign_settings,
        [editor.clone(), output]
    );
    // A Make File List output file is a session choice that Cancel keeps.
    p.begin_options();
    let file_list = FileListPreferences {
        path: Some(native("seznam.txt")),
        ..FileListPreferences::default()
    };
    p.save_file_list_preferences(file_list, &sender(&sent));
    assert_eq!(
        *p.preferences.values.foreign_settings,
        std::slice::from_ref(&editor)
    );
    p.cancel_options(&sender(&sent));
    assert_eq!(
        *p.preferences.values.foreign_settings,
        std::slice::from_ref(&editor)
    );
    let saved = sent
        .borrow()
        .iter()
        .rev()
        .find_map(|command| match command {
            Command::SavePreferences { preferences, .. } => Some(preferences.clone()),
            _ => None,
        });
    assert_eq!(*saved.unwrap().foreign_settings, [editor]);
}

#[test]
fn new_custom_commands_skip_identities_kept_from_another_system() {
    let mut p = with_kept(
        Vec::new(),
        vec![kept(
            ForeignSettingKind::UserCommand {
                position: 0,
                id: UserCommandId::new("custom-1").unwrap(),
                name: "Build".into(),
            },
            "make",
        )],
    );
    p.begin_options();
    p.new_user_command_setting();
    assert_eq!(
        p.user_command_settings.selected,
        Some(UserCommandId::new("custom-2").unwrap())
    );
}
