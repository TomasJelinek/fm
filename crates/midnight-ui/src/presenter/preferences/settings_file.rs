//! Export and Import configuration within one Options session. Backend workers
//! write, read and validate settings files; this module chooses what to export and
//! stages one validated import. Imported Options settings preview at once. The
//! session state outside Options (tabs, hot paths, window placement, panel, viewer
//! and terminal state) applies only after OK's save succeeds; Cancel discards both.
use super::options::{restore_fields, restore_session_fields};
use super::*;
use crate::presenter::settings_picker::Target;

#[cfg(test)]
#[path = "settings_file_tests.rs"]
mod tests;

#[derive(Default)]
pub(crate) struct State {
    /// The Options session that owns choices, staging and the shown result.
    session: u64,
    /// One transfer at a time. Its session may end before the backend answers.
    pending: Option<Pending>,
    /// Folder of the last chosen settings file; the next chooser starts there.
    directory: Option<PathBuf>,
    pub(crate) message: String,
    pub(crate) error: bool,
    /// An existing export destination awaiting Replace or Keep.
    replace: Option<PathBuf>,
    /// The validated import. Only its fields outside Options are read from here.
    pub(super) staged: Option<Box<UserPreferences>>,
    /// Committed imported placement awaiting the application's main window.
    window_geometry: Option<WindowGeometry>,
}

struct Pending {
    session: u64,
    path: PathBuf,
    step: Step,
}

enum Step {
    /// Chosen; the next flush with a sender submits the command.
    Export {
        overwrite: bool,
    },
    Import,
    Exporting {
        request: DesktopRequestId,
        overwrite: bool,
    },
    Importing(DesktopRequestId),
    /// Validated by the backend; the next flush stages it.
    Imported(Box<UserPreferences>),
}

impl State {
    pub(crate) fn busy(&self) -> bool {
        self.pending.is_some()
    }

    /// OK and Reset wait while an import can still change the session.
    pub(crate) fn importing(&self) -> bool {
        self.pending.as_ref().is_some_and(|pending| {
            matches!(
                pending.step,
                Step::Import | Step::Importing(_) | Step::Imported(_)
            )
        })
    }

    pub(crate) fn replace_pending(&self) -> bool {
        self.replace.is_some()
    }

    pub(crate) fn session(&self) -> u64 {
        self.session
    }

    pub(crate) fn directory(&self) -> Option<PathBuf> {
        self.directory.clone()
    }

    fn show(&mut self, message: String, error: bool) {
        self.message = message;
        self.error = error;
    }

    /// A new Options session, or the end of one: late choices and results of the
    /// previous session can no longer stage, prompt or show anything.
    pub(super) fn next_session(&mut self, retired: &mut Vec<DeferredDrop>) {
        self.session = self.session.wrapping_add(1);
        self.message.clear();
        self.error = false;
        self.replace = None;
        if let Some(staged) = self.staged.take() {
            retired.push(staged);
        }
    }

    /// Reset to defaults keeps the current session state outside Options.
    pub(super) fn discard_staged(&mut self, retired: &mut Vec<DeferredDrop>) {
        self.message.clear();
        self.error = false;
        if let Some(staged) = self.staged.take() {
            retired.push(staged);
        }
    }
}

impl PreferencesState {
    /// Exactly what OK would save now: Options previews and a staged import,
    /// without commands that were never authorized for persistence.
    pub(super) fn exportable_values(&self) -> UserPreferences {
        let mut values = self.values.clone();
        if let Some(staged) = &self.settings_file.staged {
            restore_session_fields(&mut values, staged);
        }
        self.without_session_commands(&mut values);
        values
    }
}

/// One-line feedback ends as a sentence; backend messages may omit the period.
fn sentence(text: &str) -> String {
    let text = text.trim_end();
    if text.ends_with(['.', '!', '?']) {
        text.to_owned()
    } else {
        format!("{text}.")
    }
}

fn file_name(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| path.to_string_lossy());
    crate::shown_text(&name).to_string()
}

impl Presenter {
    /// Options footer: choose a destination, then export what OK would save now.
    pub(crate) fn begin_settings_export(
        &mut self,
        owner: PathPickerOwner,
        send: &dyn Fn(Command) -> bool,
    ) {
        self.begin_settings_file(Target::SettingsExport, owner, send);
    }

    /// Options footer: choose a settings file to validate and stage.
    pub(crate) fn begin_settings_import(
        &mut self,
        owner: PathPickerOwner,
        send: &dyn Fn(Command) -> bool,
    ) {
        self.begin_settings_file(Target::SettingsImport, owner, send);
    }

    fn begin_settings_file(
        &mut self,
        target: Target,
        owner: PathPickerOwner,
        send: &dyn Fn(Command) -> bool,
    ) {
        if !self.settings_file_available() {
            return;
        }
        let state = &mut self.preferences.settings_file;
        state.replace = None;
        state.message.clear();
        state.error = false;
        self.request_settings_picker(target, owner, send);
    }

    /// Whether Export or Import may start: settings are loaded and writable, Options
    /// is open and idle, and no chooser or settings-file transfer is outstanding.
    pub(crate) fn settings_file_available(&self) -> bool {
        let options = &self.preferences.options;
        options.open
            && !options.accepting
            && !options.saving
            && self.window_geometry_ready()
            && !self.settings_picker.busy()
            && !self.preferences.settings_file.busy()
            && !self.operations.closing
    }

    pub(crate) fn settings_file_failed(&mut self, message: String) {
        self.preferences.settings_file.show(message, true);
    }

    /// A chooser answered for the current Options session.
    pub(crate) fn settings_file_picked(&mut self, export: bool, path: PathBuf) {
        if let Err(error) = validate_picked_path(&path) {
            self.settings_file_failed(crate::error_presentation::describe(&error));
            return;
        }
        let state = &mut self.preferences.settings_file;
        state.directory = path.parent().map(Path::to_path_buf);
        state.pending = Some(Pending {
            session: state.session,
            path,
            step: if export {
                Step::Export { overwrite: false }
            } else {
                Step::Import
            },
        });
    }

    /// Replace answers the existing-destination prompt with explicit consent.
    pub(crate) fn replace_settings_export(&mut self, send: &dyn Fn(Command) -> bool) {
        let state = &mut self.preferences.settings_file;
        if state.pending.is_some() || !self.preferences.options.open {
            return;
        }
        let Some(path) = state.replace.take() else {
            return;
        };
        state.message.clear();
        state.pending = Some(Pending {
            session: state.session,
            path,
            step: Step::Export { overwrite: true },
        });
        self.flush_settings_file(send);
    }

    pub(crate) fn keep_settings_export(&mut self) {
        let state = &mut self.preferences.settings_file;
        if let Some(path) = state.replace.take() {
            let message = format!("{} was kept. Settings were not exported.", file_name(&path));
            state.show(message, false);
        }
    }

    /// Submit a chosen transfer, or stage a validated import, once a sender exists.
    pub(crate) fn flush_settings_file(&mut self, send: &dyn Fn(Command) -> bool) {
        let Some(pending) = self.preferences.settings_file.pending.take() else {
            return;
        };
        let current = pending.session == self.preferences.settings_file.session
            && self.preferences.options.open
            && !self.preferences.options.saving;
        let Pending {
            session,
            path,
            step,
        } = pending;
        match step {
            // Options closed before the command was submitted: nothing happens.
            Step::Export { .. } | Step::Import if !current => {}
            Step::Export { overwrite } => {
                let request = self.desktop.request();
                let input = PreferencesExportRequest {
                    request,
                    destination: path.clone(),
                    preferences: self.preferences.exportable_values(),
                    overwrite,
                };
                let state = &mut self.preferences.settings_file;
                if send(Command::ExportPreferences(Box::new(input))) {
                    state.show("Exporting settings…".into(), false);
                    state.pending = Some(Pending {
                        session,
                        path,
                        step: Step::Exporting { request, overwrite },
                    });
                } else {
                    state.show(
                        "The settings queue is busy. Choose Export configuration again.".into(),
                        true,
                    );
                }
            }
            Step::Import => {
                let request = self.desktop.request();
                let state = &mut self.preferences.settings_file;
                if send(Command::ImportPreferences {
                    request,
                    source: path.clone(),
                }) {
                    state.show(format!("Reading {}…", file_name(&path)), false);
                    state.pending = Some(Pending {
                        session,
                        path,
                        step: Step::Importing(request),
                    });
                } else {
                    state.show(
                        "The settings queue is busy. Choose Import configuration again.".into(),
                        true,
                    );
                }
            }
            Step::Imported(imported) if current => {
                self.stage_settings_import(&path, *imported, send);
            }
            Step::Imported(imported) => self.retired.push(imported),
            step @ (Step::Exporting { .. } | Step::Importing(_)) => {
                self.preferences.settings_file.pending = Some(Pending {
                    session,
                    path,
                    step,
                });
            }
        }
    }

    pub(in crate::presenter) fn settings_exported(
        &mut self,
        request: DesktopRequestId,
        result: Result<PreferencesExportReceipt, FsError>,
    ) {
        let state = &mut self.preferences.settings_file;
        let Some(Pending {
            session,
            step:
                Step::Exporting {
                    request: expected,
                    overwrite,
                },
            ..
        }) = &state.pending
        else {
            self.retired.push(Box::new(result));
            return;
        };
        if *expected != request {
            self.retired.push(Box::new(result));
            return;
        }
        let (session, overwrite) = (*session, *overwrite);
        let Some(pending) = state.pending.take() else {
            return;
        };
        // An export outlives its dialog: report a closed session's result outside it.
        let current = session == state.session && self.preferences.options.open;
        match result {
            Ok(receipt) => {
                let mut message = format!(
                    "Exported settings to {}.",
                    crate::shown_text(&receipt.path.to_string_lossy())
                );
                for recovery in &receipt.recovery {
                    message.push_str(&format!(
                        " The replaced file was kept as {}.",
                        crate::shown_text(&recovery.to_string_lossy())
                    ));
                }
                if current {
                    state.show(message, false);
                } else {
                    self.desktop.status = message;
                }
            }
            Err(error) if error.kind == std::io::ErrorKind::AlreadyExists && !overwrite => {
                let message = format!("{} already exists. Replace it?", file_name(&pending.path));
                if current {
                    state.show(message, false);
                    state.replace = Some(pending.path);
                } else {
                    self.desktop.report(
                        "Settings were not exported",
                        format!("{} already exists.", file_name(&pending.path)),
                    );
                }
                self.retired.push(Box::new(error));
            }
            Err(error) => {
                if current {
                    let reason = sentence(&error.message);
                    state.show(format!("Could not export settings: {reason}"), true);
                } else {
                    let reason = crate::error_presentation::describe(&error);
                    self.desktop.report("Could not export settings", reason);
                }
                self.retired.push(Box::new(error));
            }
        }
    }

    pub(in crate::presenter) fn settings_imported(
        &mut self,
        request: DesktopRequestId,
        result: Result<Box<UserPreferences>, FsError>,
    ) {
        let state = &mut self.preferences.settings_file;
        let matches = state.pending.as_ref().is_some_and(
            |pending| matches!(pending.step, Step::Importing(expected) if expected == request),
        );
        let Some(pending) = state.pending.take_if(|_| matches) else {
            self.retired.push(Box::new(result));
            return;
        };
        if pending.session != state.session || !self.preferences.options.open {
            self.retired.push(Box::new(result));
            return;
        }
        match result {
            Ok(imported) => {
                state.pending = Some(Pending {
                    step: Step::Imported(imported),
                    ..pending
                });
            }
            Err(error) => {
                state.show(
                    format!(
                        "Could not import {}: {} Your settings are unchanged.",
                        file_name(&pending.path),
                        sentence(&error.message)
                    ),
                    true,
                );
                self.retired.push(Box::new(error));
            }
        }
    }

    /// Preview the imported Options settings and stage the rest for OK. A refusal
    /// changes nothing, including unfinished page drafts.
    fn stage_settings_import(
        &mut self,
        path: &Path,
        mut imported: UserPreferences,
        send: &dyn Fn(Command) -> bool,
    ) {
        let name = file_name(path);
        let refuse = |p: &mut Self, reason: &str, imported: UserPreferences| {
            p.settings_file_failed(format!(
                "Could not import {name}: {} Your settings are unchanged.",
                sentence(reason)
            ));
            p.retired.push(Box::new(imported));
        };
        // Imported hot paths get fresh identities, so a removed bookmark's late
        // callback can never act on one of them.
        let first = self
            .bookmarks()
            .iter()
            .map(|bookmark| bookmark.id.0)
            .max()
            .unwrap_or(0)
            .max(self.preferences.bookmark_generation);
        let Some(last) = first.checked_add(imported.bookmarks.len() as u64) else {
            refuse(
                self,
                "Hot path identities are exhausted; restart first.",
                imported,
            );
            return;
        };
        for (offset, bookmark) in imported.bookmarks.iter_mut().enumerate() {
            bookmark.id = BookmarkId(first + 1 + offset as u64);
        }
        // Validate key bindings before anything changes; the preview reuses them.
        if let Err(reason) = self
            .keyboard
            .sync(&imported.shortcuts, &imported.user_commands)
        {
            refuse(self, &reason, imported);
            return;
        }
        let mut values = self.preferences.values.clone();
        restore_fields(&mut values, &imported);
        self.discard_options_forms();
        self.preferences.options.error_topic = None;
        self.preferences.bookmark_generation = last;
        self.update_preferences(values, send);
        let state = &mut self.preferences.settings_file;
        if let Some(previous) = state.staged.replace(Box::new(imported)) {
            self.retired.push(previous);
        }
        state.show(
            format!(
                "Imported {name}. OK saves these settings and applies its tabs, hot paths \
                 and window placement; Cancel keeps your current settings."
            ),
            false,
        );
    }

    /// OK's save succeeded: the committed import's session state takes effect.
    /// Tabs restore through the startup restoration path on the next flush.
    pub(super) fn apply_staged_settings(&mut self) {
        let Some(staged) = self.preferences.settings_file.staged.take() else {
            return;
        };
        let filters = self.preferences.values.panel_filters != staged.panel_filters;
        restore_session_fields(&mut self.preferences.values, &staged);
        self.restore_imported_tab_sessions(&staged.tabs);
        if let Some(geometry) = &staged.window_geometry {
            self.preferences.settings_file.window_geometry = Some(geometry.clone());
        }
        if filters {
            self.search_filter_changed();
        }
        self.retired.push(staged);
    }

    /// The application moves its main window to a committed imported placement.
    pub fn take_imported_window_geometry(&mut self) -> Option<WindowGeometry> {
        self.preferences.settings_file.window_geometry.take()
    }

    pub fn imported_window_geometry_pending(&self) -> bool {
        self.preferences.settings_file.window_geometry.is_some()
    }
}
