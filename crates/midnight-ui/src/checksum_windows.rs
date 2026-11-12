//! Modeless checksum presentation. Backend result owners survive native window cancellation.
use crate::dialog_host::{CloseDisposition, DialogHost, DialogOwner, DialogSessions};
use crate::presenter::checksums::{ChecksumSaveForm, ChecksumSession, OUTPUT_ALGORITHMS};
use crate::tool_window_failure::Failures;
use crate::{ChecksumDisplayRow, ChecksumDraft, ChecksumWindow, MainWindow, Presenter, Theme};
use midnight_core::*;
use slint::{ComponentHandle, Model, ModelNotify, ModelTracker};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::{Rc, Weak},
};

struct Rows {
    state: Weak<RefCell<Presenter>>,
    job: ChecksumJobId,
    notify: ModelNotify,
    structure: Cell<(u64, usize)>,
}
impl Model for Rows {
    type Data = ChecksumDisplayRow;
    fn row_count(&self) -> usize {
        self.state.upgrade().map_or(0, |state| {
            state
                .borrow()
                .checksums
                .sessions
                .get(&self.job)
                .map_or(0, |s| s.rows().len())
        })
    }
    fn row_data(&self, index: usize) -> Option<Self::Data> {
        let state = self.state.upgrade()?;
        let p = state.borrow();
        let s = p.checksums.sessions.get(&self.job)?;
        let row = s.rows().get(index)?;
        Some(ChecksumDisplayRow {
            token: format!("{}:{}", s.generation.0, row.id.0).into(),
            name: row
                .path
                .file_name()
                .unwrap_or(row.path.as_os_str())
                .to_string_lossy()
                .as_ref()
                .into(),
            path: row
                .path
                .parent()
                .unwrap_or(&row.path)
                .to_string_lossy()
                .as_ref()
                .into(),
            status: row_status(row).into(),
            bytes: format!("{} B", p.number_locale.format_u64(row.bytes_read)).into(),
            digests: if !row.digests.is_empty() {
                row.digests
                    .iter()
                    .map(|d| format!("{}  {}", d.algorithm.label(), d.hex()))
                    .collect::<Vec<_>>()
                    .join("\n")
                    .into()
            } else {
                "".into()
            },
            sha256: digest(row, ChecksumAlgorithm::Sha256),
            sha512: digest(row, ChecksumAlgorithm::Sha512),
            md5: digest(row, ChecksumAlgorithm::Md5),
            sha1: digest(row, ChecksumAlgorithm::Sha1),
            crc32: digest(row, ChecksumAlgorithm::Crc32),
            calculated: matches!(
                row.state,
                ChecksumRowState::Calculated
                    | ChecksumRowState::Verified
                    | ChecksumRowState::Mismatch
            ),
            verified: row.state == ChecksumRowState::Verified,
            error: matches!(
                row.state,
                ChecksumRowState::SourceChanged
                    | ChecksumRowState::ReadError
                    | ChecksumRowState::Mismatch
                    | ChecksumRowState::Missing
            ),
        })
    }
    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }
}
impl Rows {
    fn update(&self, generation: u64, count: usize) {
        let old = self.structure.replace((generation, count));
        if old.0 != generation || old.1 > count {
            self.notify.reset();
        } else if old.1 < count {
            self.notify.row_added(old.1, count - old.1);
        }
    }
}
struct Record {
    window: ChecksumWindow,
    host: Rc<DialogHost>,
    shown: bool,
    rows: Rc<Rows>,
    revision: Option<u64>,
    destination_revision: u64,
    manifest_revision: u64,
    suggested: Option<std::path::PathBuf>,
}
type Windows = Rc<RefCell<HashMap<ChecksumJobId, Record>>>;

pub(crate) fn install(
    main: &MainWindow,
    state: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
) {
    let windows: Windows = Rc::new(RefCell::new(HashMap::new()));
    let sessions = RefCell::new(DialogSessions::default());
    let closing = Rc::new(Cell::new(false));
    let failures = Failures::new(
        main,
        &state,
        &send,
        &closing,
        "Checksums",
        Presenter::close_checksums,
    );
    {
        let state = state.clone();
        let send = send.clone();
        let weak = main.as_weak();
        let closing = closing.clone();
        main.on_calculate_checksums(move |pane| {
            if closing.get() {
                return;
            }
            state
                .borrow_mut()
                .request_checksums(crate::pane(pane), send.as_ref());
            if let Some(main) = weak.upgrade() {
                crate::refresh(&main, &state.borrow());
            }
        });
    }
    {
        let state = state.clone();
        let send = send.clone();
        let weak = main.as_weak();
        let closing = closing.clone();
        let windows = windows.clone();
        let failures = failures.clone();
        main.on_refresh_checksum_windows(move || {
            if closing.get() {
                return;
            }
            if let Some(main) = weak.upgrade() {
                refresh(&main, &windows, &sessions, &state, &send, &failures);
            }
        });
    }
    main.on_close_checksum_windows(move || {
        closing.set(true);
        for (id, record) in std::mem::take(&mut *windows.borrow_mut()) {
            let generation = record.rows.structure.get().0;
            if generation > 0 {
                let _ = send(Command::ReleaseChecksums {
                    job: id,
                    generation: ChecksumGeneration(generation),
                });
            }
            record.host.close(record.window.window());
        }
        // No Presenter borrow: the host shutdown timer can already own it.
        // The engine's shutdown barrier also cancels a rejected release command.
    });
}

fn refresh(
    main: &MainWindow,
    windows: &Windows,
    sessions: &RefCell<DialogSessions>,
    state: &Rc<RefCell<Presenter>>,
    send: &Rc<dyn Fn(Command) -> bool>,
    failures: &Failures<ChecksumJobId>,
) {
    let ids: Vec<_> = state.borrow().checksums.sessions.keys().copied().collect();
    let removed: Vec<_> = windows
        .borrow()
        .keys()
        .filter(|id| !ids.contains(id))
        .copied()
        .collect();
    for id in removed {
        if let Some(record) = windows.borrow_mut().remove(&id) {
            record.host.close(record.window.window());
        }
    }
    for id in ids {
        if failures.contains(id) {
            continue;
        }
        if !windows.borrow().contains_key(&id) {
            match create(main, id, sessions, state, send) {
                Ok(record) => {
                    windows.borrow_mut().insert(id, record);
                }
                Err(error) => {
                    failures.record(id, error.to_string());
                    continue;
                }
            }
        }
        let p = state.borrow();
        let Some(s) = p.checksums.sessions.get(&id) else {
            continue;
        };
        let mut records = windows.borrow_mut();
        let Some(record) = records.get_mut(&id) else {
            continue;
        };
        let w = &record.window;
        let theme = main.global::<Theme>();
        w.invoke_apply_preferences(
            theme.get_scheme(),
            theme.get_density(),
            theme.get_font_family(),
            theme.get_desktop_font_size(),
            theme.get_touch_font_size(),
        );
        crate::tooltip::synchronize_checksum(w);
        if record.revision == Some(s.revision) {
            continue;
        }
        record.revision = Some(s.revision);
        let changed = record.rows.structure.get().0 != s.generation.0;
        if changed {
            w.set_selected_row(-1);
            w.set_draft(draft(s));
            let index = OUTPUT_ALGORITHMS
                .iter()
                .position(|&a| s.algorithms.contains(a))
                .unwrap_or(0);
            w.set_output_algorithm(index as i32);
        }
        let selected = usize::try_from(w.get_output_algorithm())
            .ok()
            .and_then(|index| OUTPUT_ALGORITHMS.get(index));
        if selected.is_none_or(|&a| !s.algorithms.contains(a)) {
            w.set_output_algorithm(
                OUTPUT_ALGORITHMS
                    .iter()
                    .position(|&a| s.algorithms.contains(a))
                    .unwrap_or(0) as i32,
            );
        }
        let untouched = record
            .suggested
            .as_ref()
            .is_some_and(|path| w.get_destination() == path.to_string_lossy().as_ref());
        if record.destination_revision != s.destination_revision
            || (untouched && record.suggested.as_ref() != Some(&s.suggested))
        {
            if record.destination_revision != s.destination_revision {
                // A confirmed native choice opens the export review; cancelling
                // the picker leaves the current page and the manifest unchanged.
                w.set_page(2);
            }
            record.destination_revision = s.destination_revision;
            w.set_destination(s.suggested.to_string_lossy().as_ref().into());
        }
        record.suggested = Some(s.suggested.clone());
        if record.manifest_revision != s.manifest_revision {
            record.manifest_revision = s.manifest_revision;
            w.set_manifest_path(s.manifest_suggested.to_string_lossy().as_ref().into());
        }
        w.set_picker_busy(s.picker.is_some());
        w.set_verifying(s.verification.is_some());
        w.set_has_captured_files(
            s.tree_sources.is_some() || !s.files.selection.is_empty() || s.files.fallback.is_some(),
        );
        w.set_generation_token(s.generation.0.to_string().into());
        w.set_running(s.running);
        w.set_stopping(s.stopping);
        w.set_saving(s.save.is_some());
        w.set_save_stopping(s.save.as_ref().is_some_and(|save| save.stopping));
        w.set_error(s.error.clone().into());
        w.set_attention(
            !s.error.is_empty()
                || s.progress.errors > 0
                || s.progress.mismatched > 0
                || s.progress.missing > 0
                || s.run
                    .as_ref()
                    .is_some_and(|r| r.summary.completion != ChecksumCompletion::Completed),
        );
        w.set_progress(progress(s, &p.number_locale).into());
        w.set_details(details(s, &p.number_locale).into());
        w.set_output_available(
            Rc::new(slint::VecModel::from(
                OUTPUT_ALGORITHMS
                    .iter()
                    .map(|&a| s.algorithms.contains(a))
                    .collect::<Vec<_>>(),
            ))
            .into(),
        );
        let output_indices: Vec<i32> = OUTPUT_ALGORITHMS
            .iter()
            .enumerate()
            .filter_map(|(index, &algorithm)| {
                s.algorithms.contains(algorithm).then_some(index as i32)
            })
            .collect();
        let output_labels: Vec<slint::SharedString> = output_indices
            .iter()
            .map(|&index| OUTPUT_ALGORITHMS[index as usize].label().into())
            .collect();
        w.set_output_values(Rc::new(slint::VecModel::from(output_indices)).into());
        w.set_output_labels(Rc::new(slint::VecModel::from(output_labels)).into());
        let formats: Vec<slint::SharedString> = OUTPUT_ALGORITHMS
            .iter()
            .filter(|&&a| s.algorithms.contains(a))
            .map(|a| format!("{} (*.{})", a.label(), a.extension()).into())
            .collect();
        w.set_format_labels(Rc::new(slint::VecModel::from(formats)).into());
        let first = s
            .algorithms
            .iter()
            .next()
            .unwrap_or(ChecksumAlgorithm::Sha256);
        let counts = s.export_counts(first);
        w.set_has_output(counts.is_some_and(|counts| counts.included > 0) && !s.running);
        w.set_partial(
            counts.is_some_and(|counts| counts.excluded > 0 || counts.incomplete_discovery),
        );
        w.set_export_summary(
            counts
                .map_or_else(
                    || "Wait for calculation to finish.".into(),
                    |counts| {
                        format!(
                            "{} included · {} known excluded\n{}\n{}",
                            p.number_locale.format_u64(counts.included),
                            p.number_locale.format_u64(counts.excluded),
                            if counts.incomplete_discovery {
                                "Incomplete discovery: additional files may not have been examined."
                            } else {
                                "Discovery completed."
                            },
                            s.save_message
                        )
                    },
                )
                .into(),
        );
        record.rows.update(s.generation.0, s.rows().len());
        let shown = if record.shown {
            Ok(())
        } else {
            record.host.show(w.window()).map(|()| {
                record.shown = true;
                w.invoke_begin();
            })
        };
        drop(records);
        drop(p);
        if let Err(error) = shown {
            windows.borrow_mut().remove(&id);
            failures.record(id, error.to_string());
        }
    }
}

fn create(
    main: &MainWindow,
    id: ChecksumJobId,
    sessions: &RefCell<DialogSessions>,
    state: &Rc<RefCell<Presenter>>,
    send: &Rc<dyn Fn(Command) -> bool>,
) -> Result<Record, slint::PlatformError> {
    let session = sessions
        .borrow_mut()
        .allocate()
        .map_err(slint::PlatformError::from)?;
    let (window, host) =
        DialogHost::create_tool(session, DialogOwner::new(main), ChecksumWindow::new)?;
    let host = Rc::new(host);
    crate::tooltip::install_checksum(&window)?;
    let rows = Rc::new(Rows {
        state: Rc::downgrade(state),
        job: id,
        notify: ModelNotify::default(),
        structure: Cell::new((0, 0)),
    });
    window.set_rows(rows.clone().into());
    if let Some(s) = state.borrow().checksums.sessions.get(&id) {
        window.set_destination(s.suggested.to_string_lossy().as_ref().into());
        window.set_manifest_path(s.manifest_suggested.to_string_lossy().as_ref().into());
        if s.verify_initial {
            window.set_page(4);
        }
        window.set_draft(draft(s));
    }
    macro_rules! callback {
        ($name:ident, |$p:ident, $send:ident $(, $arg:ident)*| $body:block) => {{
            let state = state.clone(); let sender = send.clone(); let weak = main.as_weak();
            let host = host.clone();
            window.$name(move |$($arg),*| {
                if !host.is_current(session) { return; }
                { let mut state = state.borrow_mut(); let $p = &mut *state; let $send = sender.as_ref(); $body }
                if let Some(main) = weak.upgrade() { crate::refresh(&main, &state.borrow()); }
            });
        }};
    }
    callback!(on_calculate, |p, send, form| {
        match algorithms(&form) {
            Ok(algorithms) => {
                p.start_checksums(id, algorithms, form.recursive, send);
            }
            Err(error) => {
                if let Some(s) = p.checksums.sessions.get_mut(&id) {
                    s.error = error.into();
                    s.revision = s.revision.wrapping_add(1);
                }
            }
        }
    });
    callback!(on_stop, |p, send| {
        p.stop_checksums(id, send);
    });
    {
        let host = host.clone();
        let state = state.clone();
        let send = send.clone();
        let weak = window.as_weak();
        let main = main.as_weak();
        window.on_close_session(move || {
            let Some(window) = weak.upgrade() else { return };
            host.request_close(window.window(), || {
                let mut p = state.borrow_mut();
                p.close_checksums(id, send.as_ref());
                if p.checksums.sessions.contains_key(&id) {
                    CloseDisposition::KeepOpen
                } else {
                    CloseDisposition::Hide
                }
            });
            if let Some(main) = main.upgrade() {
                crate::refresh(&main, &state.borrow());
            }
        });
    }
    callback!(on_stop_save, |p, send| {
        p.cancel_checksum_save(id, send);
    });
    {
        let weak = window.as_weak();
        callback!(on_verify_manifest, |p, send, path| {
            if p.start_checksum_verification(id, &path, send)
                && let Some(w) = weak.upgrade()
            {
                w.set_page(0);
            }
        });
    }
    {
        let weak = window.as_weak();
        callback!(on_browse_manifest, |p, send, verify, index| {
            if let Some(w) = weak.upgrade()
                && let Some(&algorithm) = usize::try_from(index)
                    .ok()
                    .and_then(|i| OUTPUT_ALGORITHMS.get(i))
            {
                let owner = crate::picker_window::native_path_picker_owner(w.window());
                p.request_checksum_picker(id, verify, algorithm, owner, send);
            }
        });
    }
    {
        let weak = window.as_weak();
        let state = state.clone();
        let main_weak = main.as_weak();
        let host = host.clone();
        window.on_output_changed(move |index| {
            if !host.is_current(session) {
                return;
            }
            let Some(w) = weak.upgrade() else { return };
            let Some(&algorithm) = usize::try_from(index)
                .ok()
                .and_then(|i| OUTPUT_ALGORITHMS.get(i))
            else {
                return;
            };
            // A backend refresh may update this property while it reads Presenter.
            // Only a user change should rewrite an untouched generated suggestion.
            let Ok(mut p) = state.try_borrow_mut() else {
                return;
            };
            let Some(s) = p.checksums.sessions.get_mut(&id) else {
                return;
            };
            if w.get_destination() == s.suggested.to_string_lossy().as_ref()
                && s.suggested
                    .file_stem()
                    .is_some_and(|name| name == "checksums")
            {
                s.suggested.set_extension(algorithm.extension());
                s.revision = s.revision.wrapping_add(1);
                w.set_destination(s.suggested.to_string_lossy().as_ref().into());
                drop(p);
                if let Some(main) = main_weak.upgrade() {
                    crate::refresh(&main, &state.borrow());
                }
            }
        });
    }
    callback!(on_copy_result, |p, send, token, index, whole| {
        if let Some(&algorithm) = usize::try_from(index)
            .ok()
            .and_then(|i| OUTPUT_ALGORITHMS.get(i))
        {
            p.copy_checksum(id, &token, algorithm, whole, send);
        }
    });
    callback!(on_copy_details, |p, send| {
        if let Some(s) = p.checksums.sessions.get(&id) {
            let text = format!(
                "{}\n\n{}\n{}",
                progress(s, &p.number_locale),
                s.error,
                details(s, &p.number_locale)
            );
            p.copy_viewer_text(text, send);
        }
    });
    callback!(
        on_save_manifest,
        |p, send, token, index, destination, acknowledge| {
            if let (Ok(generation), Some(&algorithm)) = (
                token.parse::<u64>(),
                usize::try_from(index)
                    .ok()
                    .and_then(|i| OUTPUT_ALGORITHMS.get(i)),
            ) {
                p.save_checksum_manifest(
                    id,
                    ChecksumSaveForm {
                        generation,
                        algorithm,
                        destination: destination.to_string(),
                        acknowledge,
                    },
                    send,
                );
            }
        }
    );
    let weak = window.as_weak();
    window.window().on_close_requested(move || {
        if let Some(w) = weak.upgrade() {
            w.invoke_close_session();
        }
        slint::CloseRequestResponse::KeepWindowShown
    });
    Ok(Record {
        window,
        host,
        shown: false,
        rows,
        revision: None,
        destination_revision: 0,
        manifest_revision: 0,
        suggested: state
            .borrow()
            .checksums
            .sessions
            .get(&id)
            .map(|s| s.suggested.clone()),
    })
}

fn algorithms(draft: &ChecksumDraft) -> Result<ChecksumAlgorithms, &'static str> {
    ChecksumAlgorithms::new(
        &[
            (draft.sha256, ChecksumAlgorithm::Sha256),
            (draft.sha512, ChecksumAlgorithm::Sha512),
            (draft.md5, ChecksumAlgorithm::Md5),
            (draft.sha1, ChecksumAlgorithm::Sha1),
            (draft.crc32, ChecksumAlgorithm::Crc32),
        ]
        .into_iter()
        .filter_map(|(enabled, a)| enabled.then_some(a))
        .collect::<Vec<_>>(),
    )
}
fn draft(s: &ChecksumSession) -> ChecksumDraft {
    ChecksumDraft {
        md5: s.algorithms.contains(ChecksumAlgorithm::Md5),
        sha1: s.algorithms.contains(ChecksumAlgorithm::Sha1),
        sha256: s.algorithms.contains(ChecksumAlgorithm::Sha256),
        sha512: s.algorithms.contains(ChecksumAlgorithm::Sha512),
        crc32: s.algorithms.contains(ChecksumAlgorithm::Crc32),
        recursive: s.recursive,
    }
}
fn digest(row: &ChecksumRow, algorithm: ChecksumAlgorithm) -> slint::SharedString {
    row.digest(algorithm)
        .map_or_else(Default::default, |d| d.hex().into())
}
fn row_status(row: &ChecksumRow) -> String {
    let label = match row.state {
        ChecksumRowState::Calculated => "Calculated",
        ChecksumRowState::SourceChanged => "Source changed",
        ChecksumRowState::ReadError => "Read error",
        ChecksumRowState::Skipped => "Skipped",
        ChecksumRowState::Cancelled => "Cancelled",
        ChecksumRowState::Verified => "Verified",
        ChecksumRowState::Mismatch => "Mismatch",
        ChecksumRowState::Missing => "Missing",
    };
    row.error
        .as_ref()
        .map_or_else(|| label.into(), |e| format!("{label}: {}", e.message))
}
fn progress(s: &ChecksumSession, locale: &NumberLocale) -> String {
    let status = if !s.error.is_empty() {
        "Error · see Details"
    } else if s.stopping {
        "Stopping…"
    } else if s.running {
        if s.verification.is_some() {
            "Verifying…"
        } else {
            "Calculating…"
        }
    } else {
        match s.run.as_ref().map(|run| run.summary.completion) {
            Some(ChecksumCompletion::Completed) => "Completed",
            Some(ChecksumCompletion::CompletedWithErrors) => {
                if s.verification.is_some() {
                    "Completed with differences"
                } else {
                    "Completed with exclusions"
                }
            }
            Some(ChecksumCompletion::Incomplete) => "Incomplete",
            Some(ChecksumCompletion::Cancelled) => "Cancelled",
            Some(ChecksumCompletion::Failed) => "Failed",
            None => "Ready",
        }
    };
    if s.verification.is_some() {
        return format!(
            "{status} · {} verified · {} mismatched · {} missing · {} B read",
            locale.format_u64(s.progress.verified),
            locale.format_u64(s.progress.mismatched),
            locale.format_u64(s.progress.missing),
            locale.format_u64(s.progress.bytes_read)
        );
    }
    format!(
        "{status} · {} calculated · {} B read",
        locale.format_u64(s.progress.calculated),
        locale.format_u64(s.progress.bytes_read)
    )
}
fn details(s: &ChecksumSession, locale: &NumberLocale) -> String {
    let mut text = format!(
        "Captured folder\n{}\n\n{} discovered · {} skipped · {} errors\n{}",
        s.directory.display(),
        locale.format_u64(s.progress.discovered),
        locale.format_u64(s.progress.skipped),
        locale.format_u64(s.progress.errors),
        s.save_message
    );
    if let Some(path) = &s.progress.current {
        text.push_str(&format!("\n\nCurrent file\n{}", path.display()));
    }
    if let Some(path) = &s.verification {
        text.push_str(&format!("\n\nChecksum file\n{}", path.display()));
    }
    if let Some(run) = &s.run {
        if !run.summary.reached_budgets.is_empty() {
            text.push_str(&format!(
                "\n\nReached resource limits: {:?}",
                run.summary.reached_budgets
            ));
        }
        for error in &run.summary.diagnostics {
            text.push_str(&format!(
                "\n\n{}",
                crate::error_presentation::describe(error)
            ));
        }
    }
    text
}
