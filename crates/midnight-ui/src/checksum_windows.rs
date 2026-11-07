//! Modeless checksum presentation. Backend result owners survive native window cancellation.
use crate::presenter::checksums::{ChecksumSaveForm, ChecksumSession};
use crate::tool_window_failure::Failures;
use crate::{ChecksumDisplayRow, ChecksumDraft, ChecksumWindow, MainWindow, Presenter, Theme};
use midnight_core::*;
use slint::{ComponentHandle, Model, ModelNotify, ModelTracker};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::{Rc, Weak},
};

const OUTPUT_ALGORITHMS: [ChecksumAlgorithm; 4] = [
    ChecksumAlgorithm::Sha256,
    ChecksumAlgorithm::Sha512,
    ChecksumAlgorithm::Md5,
    ChecksumAlgorithm::Sha1,
];

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
            digests: if row.state == ChecksumRowState::Calculated {
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
            calculated: row.state == ChecksumRowState::Calculated,
            error: matches!(
                row.state,
                ChecksumRowState::SourceChanged | ChecksumRowState::ReadError
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
    rows: Rc<Rows>,
    revision: Option<u64>,
}
type Windows = Rc<RefCell<HashMap<ChecksumJobId, Record>>>;

pub(crate) fn install(
    main: &MainWindow,
    state: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
) {
    let windows: Windows = Rc::new(RefCell::new(HashMap::new()));
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
                refresh(&main, &windows, &state, &send, &failures);
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
            let _ = record.window.hide();
        }
        // No Presenter borrow: the host shutdown timer can already own it.
        // The engine's shutdown barrier also cancels a rejected release command.
    });
}

fn refresh(
    main: &MainWindow,
    windows: &Windows,
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
            let _ = record.window.hide();
        }
    }
    for id in ids {
        if failures.contains(id) {
            continue;
        }
        if !windows.borrow().contains_key(&id) {
            match create(main, id, state, send) {
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
        let mut windows = windows.borrow_mut();
        let Some(record) = windows.get_mut(&id) else {
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
        w.set_generation_token(s.generation.0.to_string().into());
        w.set_running(s.running);
        w.set_stopping(s.stopping);
        w.set_saving(s.save.is_some());
        w.set_save_stopping(s.save.as_ref().is_some_and(|save| save.stopping));
        w.set_error(s.error.clone().into());
        w.set_attention(
            !s.error.is_empty()
                || s.progress.errors > 0
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
    }
}

fn create(
    main: &MainWindow,
    id: ChecksumJobId,
    state: &Rc<RefCell<Presenter>>,
    send: &Rc<dyn Fn(Command) -> bool>,
) -> Result<Record, slint::PlatformError> {
    let window = ChecksumWindow::new()?;
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
        window.set_draft(draft(s));
    }
    macro_rules! callback {
        ($name:ident, |$p:ident, $send:ident $(, $arg:ident)*| $body:block) => {{
            let state = state.clone(); let sender = send.clone(); let weak = main.as_weak();
            window.$name(move |$($arg),*| {
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
    callback!(on_close_session, |p, send| {
        p.close_checksums(id, send);
    });
    callback!(on_stop_save, |p, send| {
        p.cancel_checksum_save(id, send);
    });
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
    window.show()?;
    window.invoke_begin();
    Ok(Record {
        window,
        rows,
        revision: None,
    })
}

fn algorithms(draft: &ChecksumDraft) -> Result<ChecksumAlgorithms, &'static str> {
    ChecksumAlgorithms::new(
        &[
            (draft.sha256, ChecksumAlgorithm::Sha256),
            (draft.sha512, ChecksumAlgorithm::Sha512),
            (draft.md5, ChecksumAlgorithm::Md5),
            (draft.sha1, ChecksumAlgorithm::Sha1),
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
        "Calculating…"
    } else {
        match s.run.as_ref().map(|run| run.summary.completion) {
            Some(ChecksumCompletion::Completed) => "Completed",
            Some(ChecksumCompletion::CompletedWithErrors) => "Completed with exclusions",
            Some(ChecksumCompletion::Incomplete) => "Incomplete",
            Some(ChecksumCompletion::Cancelled) => "Cancelled",
            Some(ChecksumCompletion::Failed) => "Failed",
            None => "Ready",
        }
    };
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
