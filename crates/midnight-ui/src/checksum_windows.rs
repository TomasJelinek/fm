//! Modeless checksum presentation. Backend result owners survive native window cancellation.
use crate::dialog_host::{CloseDisposition, DialogHost, DialogOwner, DialogSessions};
use crate::presenter::checksums::{ChecksumSession, OUTPUT_ALGORITHMS};
use crate::tool_window_failure::Failures;
use crate::{ChecksumDisplayRow, ChecksumDraft, ChecksumWindow, MainWindow, Presenter, Theme};
use midnight_core::*;
use slint::{ComponentHandle, Model, ModelNotify, ModelTracker};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::{Rc, Weak},
};

/// Finished rows first, then the running calculation's listed files that have no
/// row yet: the file being read, then the waiting ones.
struct Rows {
    state: Weak<RefCell<Presenter>>,
    job: ChecksumJobId,
    notify: ModelNotify,
    /// Generation, finished rows and listed files without a row, as last published.
    structure: Cell<(u64, usize, usize)>,
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
                .map_or(0, |s| s.rows().len() + s.pending_count())
        })
    }
    fn row_data(&self, index: usize) -> Option<Self::Data> {
        let state = self.state.upgrade()?;
        let p = state.borrow();
        let s = p.checksums.sessions.get(&self.job)?;
        let Some(row) = s.rows().get(index) else {
            let file = s.pending_at(index - s.rows().len())?;
            return Some(pending_row(s, file, index, &p.number_locale));
        };
        Some(ChecksumDisplayRow {
            token: format!("{}:{}", s.generation.0, row.id.0).into(),
            name: crate::shown_text(
                &row.path
                    .file_name()
                    .unwrap_or(row.path.as_os_str())
                    .to_string_lossy(),
            ),
            path: crate::shown_text(&row.path.parent().unwrap_or(&row.path).to_string_lossy()),
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
            current: false,
        })
    }
    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }
}
impl Rows {
    fn update(&self, generation: u64, finished: usize, pending: usize) {
        let (old_generation, old_finished, old_pending) =
            self.structure.replace((generation, finished, pending));
        let (old_total, total) = (old_finished + old_pending, finished + pending);
        if old_generation != generation || finished < old_finished {
            self.notify.reset();
        } else if old_pending == 0 && pending == 0 {
            // Rows only append: verification, or nothing was listed.
            if total > old_total {
                self.notify.row_added(old_total, total - old_total);
            }
        } else if total == old_total {
            // Listed files became rows in place, and the file being read changed.
            for row in old_finished..(finished + 1).min(total) {
                self.notify.row_changed(row);
            }
        } else if finished == old_finished && total > old_total {
            // More files were listed.
            self.notify.row_added(old_total, total - old_total);
        } else {
            // A row for an entry that was not listed moved the waiting files.
            self.notify.reset();
        }
    }
}

/// A listed file without a row yet: waiting, or being read.
fn pending_row(
    s: &ChecksumSession,
    file: &ChecksumPlannedFile,
    index: usize,
    locale: &NumberLocale,
) -> ChecksumDisplayRow {
    let current = s.progress.current.as_deref() == Some(file.path.as_path());
    let status = if !current {
        "Waiting".to_owned()
    } else if let Some(size) = file.size.filter(|size| *size > 0) {
        format!(
            "Reading · {} %",
            s.progress.current_read.min(size) * 100 / size
        )
    } else {
        "Reading".to_owned()
    };
    ChecksumDisplayRow {
        // Not a row identity: copy actions ignore it.
        token: format!("{}:listed:{index}", s.generation.0).into(),
        name: crate::shown_text(
            &file
                .path
                .file_name()
                .unwrap_or(file.path.as_os_str())
                .to_string_lossy(),
        ),
        path: crate::shown_text(&file.path.parent().unwrap_or(&file.path).to_string_lossy()),
        status: status.into(),
        bytes: if current {
            format!("{} B", locale.format_u64(s.progress.current_read)).into()
        } else {
            Default::default()
        },
        current,
        ..Default::default()
    }
}
struct Record {
    window: ChecksumWindow,
    host: Rc<DialogHost>,
    shown: bool,
    rows: Rc<Rows>,
    revision: Option<u64>,
    manifest_revision: u64,
    suggested: Option<std::path::PathBuf>,
    /// Algorithm set behind the published output models. `ModelRc` compares by
    /// identity, so republishing equal models would reset both output choosers
    /// and every visible result row on each checksum batch.
    output_algorithms: Option<ChecksumAlgorithms>,
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
        // Copy details formats the session again, with raw paths.
        w.set_error(crate::shown_text(&s.error));
        w.set_issue(crate::shown_text(&s.issue().unwrap_or_default()));
        w.set_save_status(crate::shown_text(&s.save_status(&p.number_locale)));
        w.set_details(crate::shown_text(&s.details(&p.number_locale)));
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
        let (fraction, listing) = fraction(s);
        w.set_progress_fraction(fraction);
        w.set_progress_listing(listing);
        w.set_save_prompt(
            s.save_prompt
                .as_ref()
                .map_or_else(Default::default, |prompt| {
                    crate::shown_text(&prompt.message)
                }),
        );
        w.set_save_prompt_action(
            if s.save_prompt
                .as_ref()
                .is_some_and(|prompt| prompt.overwrite)
            {
                "Replace"
            } else {
                "Save"
            }
            .into(),
        );
        if record.output_algorithms != Some(s.algorithms) {
            record.output_algorithms = Some(s.algorithms);
            publish_outputs(w, s.algorithms);
        }
        let first = s
            .algorithms
            .iter()
            .next()
            .unwrap_or(ChecksumAlgorithm::Sha256);
        let counts = s.export_counts(first);
        w.set_has_output(counts.is_some_and(|counts| counts.included > 0) && !s.running);
        record
            .rows
            .update(s.generation.0, s.rows().len(), s.pending_count());
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
        structure: Cell::new((0, 0, 0)),
    });
    window.set_rows(rows.clone().into());
    if let Some(s) = state.borrow().checksums.sessions.get(&id) {
        window.set_draft(draft(s));
        window.set_output_algorithm(
            OUTPUT_ALGORITHMS
                .iter()
                .position(|&a| s.algorithms.contains(a))
                .unwrap_or(0) as i32,
        );
        window.set_manifest_path(s.manifest_suggested.to_string_lossy().as_ref().into());
        if s.verify_initial {
            window.set_page(1);
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
    // Enter in a verification's results: the same checksum file again.
    callback!(on_verify_again, |p, send| {
        p.repeat_checksum_verification(id, send);
    });
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
    callback!(on_copy_result, |p, send, token, index, whole| {
        if let Some(&algorithm) = usize::try_from(index)
            .ok()
            .and_then(|i| OUTPUT_ALGORITHMS.get(i))
        {
            p.copy_checksum(id, &token, algorithm, whole, send);
        }
    });
    callback!(on_copy_checksum_file, |p, send, index| {
        if let Some(&algorithm) = usize::try_from(index)
            .ok()
            .and_then(|i| OUTPUT_ALGORITHMS.get(i))
        {
            p.copy_checksum_file(id, algorithm, send);
        }
    });
    callback!(on_confirm_save, |p, send| {
        p.confirm_checksum_save(id, send);
    });
    callback!(on_cancel_save_prompt, |p, send| {
        let _ = send;
        p.dismiss_checksum_save_prompt(id);
    });
    callback!(on_copy_details, |p, send| {
        // The presenter's text with exact paths, not the display-safe copy shown.
        if let Some(details) = p
            .checksums
            .sessions
            .get(&id)
            .map(|s| s.details(&p.number_locale))
        {
            p.copy_viewer_text(details, send);
        }
    });
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
        manifest_revision: 0,
        output_algorithms: None,
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
/// The output choosers and result columns for a captured algorithm set.
fn publish_outputs(w: &ChecksumWindow, algorithms: ChecksumAlgorithms) {
    w.set_output_available(
        Rc::new(slint::VecModel::from(
            OUTPUT_ALGORITHMS
                .iter()
                .map(|&a| algorithms.contains(a))
                .collect::<Vec<_>>(),
        ))
        .into(),
    );
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
        "Error"
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
    if s.running && s.error.is_empty() && !s.stopping {
        if s.progress.listing {
            return format!(
                "Listing files… · {} found",
                locale.format_u64(s.progress.planned)
            );
        }
        if s.progress.planned > 0 {
            // A file that could not be read is done too, as for the bar (`fraction`).
            let done = s.progress.calculated + s.progress.errors + s.progress.skipped;
            return format!(
                "{status} · {} of {} · {} of {} B read",
                locale.format_u64(done),
                crate::job_presentation::counted(s.progress.planned, "file", "files", locale),
                locale.format_u64(s.progress.bytes_read),
                locale.format_u64(s.progress.planned_bytes)
            );
        }
    }
    format!(
        "{status} · {} calculated · {} B read",
        locale.format_u64(s.progress.calculated),
        locale.format_u64(s.progress.bytes_read)
    )
}
/// The running calculation's share done, by bytes (by files when the listed files
/// are all empty), and whether it is still listing. A negative share hides the bar.
fn fraction(s: &ChecksumSession) -> (f32, bool) {
    let progress = &s.progress;
    if !s.running || s.verification.is_some() {
        return (-1.0, false);
    }
    if progress.listing {
        return (0.0, true);
    }
    let (done, total) = if progress.planned_bytes > 0 {
        (progress.bytes_read, progress.planned_bytes)
    } else {
        (
            progress.calculated + progress.errors + progress.skipped,
            progress.planned,
        )
    };
    if total == 0 {
        return (-1.0, false);
    }
    ((done as f64 / total as f64).min(1.0) as f32, false)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_updates_keep_the_output_models_of_an_unchanged_algorithm_set() {
        let h = crate::test_harness::Harness::with_location(
            UserPreferences::default(),
            std::env::temp_dir(),
        );
        let state = h.presenter.clone();
        let send: Rc<dyn Fn(Command) -> bool> = Rc::new(|_| true);
        state
            .borrow_mut()
            .request_checksums(PaneId::Left, send.as_ref());
        let id = *state
            .borrow()
            .checksums
            .sessions
            .keys()
            .next()
            .expect("an empty panel opens a checksum session");
        // Drive this module's refresh directly to reach the window it creates.
        let windows: Windows = Rc::default();
        let sessions = RefCell::new(DialogSessions::default());
        let closing = Rc::new(Cell::new(false));
        let failures = Failures::new(
            &h.window,
            &state,
            &send,
            &closing,
            "Checksums",
            Presenter::close_checksums,
        );
        let update = || refresh(&h.window, &windows, &sessions, &state, &send, &failures);
        let outputs = || {
            let records = windows.borrow();
            let window = &records[&id].window;
            window.get_output_available()
        };
        update();
        let published = outputs();
        let algorithms = state.borrow().checksums.sessions[&id].algorithms;
        assert!(
            state
                .borrow_mut()
                .start_checksums(id, algorithms, false, send.as_ref())
        );
        update();
        let generation = state.borrow().checksums.sessions[&id].generation;
        for discovered in 1..=3 {
            state
                .borrow_mut()
                .apply(Event::Checksum(Box::new(ChecksumEvent {
                    job: id,
                    generation,
                    update: ChecksumUpdate::Progress(ChecksumProgress {
                        discovered,
                        ..Default::default()
                    }),
                })));
            update();
            // ModelRc equality is identity: a republished model resets the
            // algorithm menu and every visible result row.
            assert!(
                outputs() == published,
                "progress {discovered} republished the output models"
            );
        }
        // Verification narrows the session to the manifest's algorithm.
        assert!(!algorithms.contains(ChecksumAlgorithm::Md5));
        state
            .borrow_mut()
            .apply(Event::Checksum(Box::new(ChecksumEvent {
                job: id,
                generation,
                update: ChecksumUpdate::Verifying(ChecksumAlgorithm::Md5),
            })));
        update();
        let republished = outputs();
        assert!(republished != published, "a new algorithm set republishes");
        let available: Vec<bool> = republished.iter().collect();
        assert_eq!(available, [false, false, true, false, false]);
    }

    /// "1 of 1 file", not "0 of 1 files": one listed file in the singular, and a file
    /// that could not be read counts as done, as `fraction` counts it.
    #[test]
    fn the_status_line_counts_listed_files_as_the_bar_does() {
        let mut p = Presenter::new(std::env::temp_dir(), std::env::temp_dir());
        let send = |_| true;
        p.request_checksums(PaneId::Left, &send);
        let id = *p.checksums.sessions.keys().next().unwrap();
        let algorithms = p.checksums.sessions[&id].algorithms;
        assert!(p.start_checksums(id, algorithms, false, &send));
        let generation = p.checksums.sessions[&id].generation;
        let locale = NumberLocale::default();
        for (errors, planned, wanted) in [
            (1, 1, "Calculating… · 1 of 1 file · 0 of 8 B read"),
            (0, 2, "Calculating… · 0 of 2 files · 0 of 8 B read"),
        ] {
            p.apply(Event::Checksum(Box::new(ChecksumEvent {
                job: id,
                generation,
                update: ChecksumUpdate::Progress(ChecksumProgress {
                    planned,
                    planned_bytes: 8,
                    errors,
                    ..Default::default()
                }),
            })));
            assert_eq!(progress(&p.checksums.sessions[&id], &locale), wanted);
        }
    }
}
