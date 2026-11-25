//! Modeless checksum presentation. Backend result owners survive native window cancellation.
use crate::dialog_host::{CloseDisposition, DialogHost, DialogOwner, DialogSessions};
use crate::presenter::checksums::{
    ChecksumSession, OUTPUT_ALGORITHMS,
    selection::{listed_slot, row_slot},
};
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
            let (place, file) = s.pending_at(index - s.rows().len())?;
            return Some(pending_row(s, place, file, &p.number_locale));
        };
        // Display text is formatted here, for the shown rows only; rows keep none.
        let path = row.path();
        let selected = row_slot(row.id.0).is_some_and(|slot| s.selection.is_selected(slot));
        Some(ChecksumDisplayRow {
            token: format!("{}:{}", s.generation.0, row.id.0).into(),
            name: crate::shown_text(
                &path
                    .file_name()
                    .unwrap_or(path.as_os_str())
                    .to_string_lossy(),
            ),
            path: crate::shown_text(&path.parent().unwrap_or(&path).to_string_lossy()),
            status: row_status(&row).into(),
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
            sha256: digest(&row, ChecksumAlgorithm::Sha256),
            sha512: digest(&row, ChecksumAlgorithm::Sha512),
            md5: digest(&row, ChecksumAlgorithm::Md5),
            sha1: digest(&row, ChecksumAlgorithm::Sha1),
            crc32: digest(&row, ChecksumAlgorithm::Crc32),
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
            selected,
        })
    }
    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }
}
/// Above this many rows whose marks changed the list is reset instead of told of each
/// row (`Rows::marks_changed`). One `row_changed` costs a notification of every view
/// even for a row out of sight, so Select all at 100,000 rows sent 100,000 of them; a
/// reset is one notification, after which the list reads again only the rows it shows
/// (a few dozen), keeping its scroll position. A click or a key changes fewer rows.
const MARKS_RESET: usize = 256;

impl Rows {
    /// The shown rows whose marks changed: each one is read again where it is shown, or,
    /// past `MARKS_RESET` rows, every shown row through one reset.
    fn marks_changed(&self, changed: &[std::ops::Range<usize>]) {
        let count = self.row_count();
        let rows: usize = changed
            .iter()
            .map(|range| range.end.min(count).saturating_sub(range.start))
            .sum();
        if rows > MARKS_RESET {
            self.notify.reset();
            return;
        }
        for range in changed {
            for row in range.start..range.end.min(count) {
                self.notify.row_changed(row);
            }
        }
    }
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

/// A listed file without a row yet: waiting, or being read. Its token names its place in
/// the listing, its identity until its row arrives.
fn pending_row(
    s: &ChecksumSession,
    place: usize,
    file: &ChecksumPlannedFile,
    locale: &NumberLocale,
) -> ChecksumDisplayRow {
    let path = s
        .pending_base()
        .map_or_else(|| file.name.to_path_buf(), |base| base.join(&file.name));
    let current = s.progress.current.as_deref() == Some(path.as_path());
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
        token: format!("{}:listed:{place}", s.generation.0).into(),
        name: crate::shown_text(
            &path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy(),
        ),
        path: crate::shown_text(&path.parent().unwrap_or(&path).to_string_lossy()),
        status: status.into(),
        bytes: if current {
            format!("{} B", locale.format_u64(s.progress.current_read)).into()
        } else {
            Default::default()
        },
        current,
        selected: listed_slot(place).is_some_and(|slot| s.selection.is_selected(slot)),
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
            let pane = crate::pane(pane);
            let send = send.as_ref();
            state.borrow_mut().panel_command(pane, send, |p| {
                p.request_checksums(pane, send);
            });
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
        publish_selection(w, s, &p.number_locale);
        // The row being calculated: the earliest listed file without its row.
        w.set_active_row(if s.running && s.pending_count() > 0 {
            i32::try_from(s.rows().len()).unwrap_or(i32::MAX)
        } else {
            -1
        });
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
                p.request_checksum_picker(
                    id,
                    verify,
                    algorithm,
                    &w.get_manifest_path(),
                    owner,
                    send,
                );
            }
        });
    }
    // The context menu: its row's token, captured when it opened, and whether it acts on
    // the selection (the row was selected then).
    callback!(on_copy_result, |p, send, token, index, whole, selection| {
        if let Some(&algorithm) = usize::try_from(index)
            .ok()
            .and_then(|i| OUTPUT_ALGORITHMS.get(i))
        {
            p.copy_checksum(id, &token, algorithm, whole, selection, send);
        }
    });
    // Ctrl+C and Ctrl+Shift+C in the list.
    callback!(on_copy_selection, |p, send, index, whole| {
        if let Some(&algorithm) = usize::try_from(index)
            .ok()
            .and_then(|i| OUTPUT_ALGORITHMS.get(i))
        {
            p.copy_checksums(id, algorithm, whole, send);
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
    install_selection(&window, &host, session, state, &rows, id);
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
/// The list's selection callbacks. A click or a selection key changes only the marks, the
/// cursor and what Copy can do: the shown rows whose marks changed are read again and
/// those few properties set, without a window refresh.
fn install_selection(
    window: &ChecksumWindow,
    host: &Rc<DialogHost>,
    session: crate::dialog_host::DialogSession,
    state: &Rc<RefCell<Presenter>>,
    rows: &Rc<Rows>,
    id: ChecksumJobId,
) {
    type Change = dyn Fn(&mut ChecksumSession) -> Option<crate::list_selection::Changed>;
    let select: Rc<dyn Fn(&Change)> = {
        let (state, rows, host, weak) =
            (state.clone(), rows.clone(), host.clone(), window.as_weak());
        Rc::new(move |change: &Change| {
            if !host.is_current(session) {
                return;
            }
            let changed = {
                let mut p = state.borrow_mut();
                let Some(s) = p.checksums.sessions.get_mut(&id) else {
                    return;
                };
                change(s)
            };
            let Some(changed) = changed else {
                return;
            };
            // The model reads the presenter again: no borrow may be held here.
            rows.marks_changed(&changed);
            let p = state.borrow();
            if let (Some(w), Some(s)) = (weak.upgrade(), p.checksums.sessions.get(&id)) {
                publish_selection(&w, s, &p.number_locale);
            }
        })
    };
    let index = |value: i32| usize::try_from(value).ok();
    let pick = select.clone();
    window.on_pick_row(move |row, control, shift| {
        if let Some(row) = index(row) {
            pick(&move |s| s.click_row(row, control, shift));
        }
    });
    let reach = select.clone();
    window.on_move_cursor(move |row, extend, keep| {
        if let Some(row) = index(row) {
            reach(&move |s| s.move_cursor(row, extend, keep));
        }
    });
    let toggle = select.clone();
    window.on_toggle_row(move |advance| toggle(&move |s| s.toggle_cursor(advance)));
    let all = select.clone();
    window.on_select_all(move || all(&|s| Some(s.select_all_rows())));
    window.on_deselect_all(move || select(&|s| Some(s.deselect_all_rows())));
}

/// The cursor and what the selection can copy: the lines Copy writes (selected calculated
/// rows) and the rows with a checksum for Ctrl+C and the context menu.
fn publish_selection(w: &ChecksumWindow, s: &ChecksumSession, locale: &NumberLocale) {
    let int = |value: u64| i32::try_from(value).unwrap_or(i32::MAX);
    w.set_cursor_row(
        s.cursor_row()
            .map_or(-1, |row| i32::try_from(row).unwrap_or(i32::MAX)),
    );
    w.set_selection_empty(s.selection.is_empty());
    let (lines, hashes) = s.selection_counts();
    w.set_copy_lines(int(lines));
    w.set_copy_lines_text(locale.format_u64(lines).into());
    w.set_selected_hashes(int(hashes));
    w.set_selected_hashes_text(locale.format_u64(hashes).into());
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

    /// The window learns which row is being calculated (the first listed file without a
    /// row, none once the run ends), the cursor, and whether anything is selected and
    /// what Copy can write; a selection change notifies only the rows it marks.
    #[test]
    fn the_window_follows_the_first_waiting_file_and_publishes_the_selection() {
        let h = crate::test_harness::Harness::with_location(
            UserPreferences::default(),
            std::env::temp_dir(),
        );
        let state = h.presenter.clone();
        let send: Rc<dyn Fn(Command) -> bool> = Rc::new(|_| true);
        state
            .borrow_mut()
            .request_checksums(PaneId::Left, send.as_ref());
        let id = *state.borrow().checksums.sessions.keys().next().unwrap();
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
        let window = || windows.borrow()[&id].window.clone_strong();
        update();
        assert_eq!(window().get_active_row(), -1, "nothing runs");
        let algorithms = state.borrow().checksums.sessions[&id].algorithms;
        assert!(
            state
                .borrow_mut()
                .start_checksums(id, algorithms, false, send.as_ref())
        );
        let root = std::env::temp_dir();
        let apply = |update| {
            let generation = state.borrow().checksums.sessions[&id].generation;
            state
                .borrow_mut()
                .apply(Event::Checksum(Box::new(ChecksumEvent {
                    job: id,
                    generation,
                    update,
                })));
        };
        apply(ChecksumUpdate::Planned(std::sync::Arc::new(
            ChecksumPlanned {
                base: root.as_path().into(),
                files: ["a", "b", "c"]
                    .map(|name| ChecksumPlannedFile {
                        name: std::path::Path::new(name).into(),
                        size: Some(1),
                    })
                    .into(),
            },
        )));
        update();
        assert_eq!(window().get_active_row(), 0);
        let row = |id: u64, name: &str| ChecksumRow {
            id: ChecksumRowId(id),
            name: std::path::Path::new(name).into(),
            identity: None,
            kind: Some(EntryKind::File),
            state: ChecksumRowState::Calculated,
            bytes_read: 1,
            digests: ChecksumDigests::new(&[(ChecksumAlgorithm::Sha256, &[1; 32])]).unwrap(),
            error: None,
        };
        apply(ChecksumUpdate::Batch(std::sync::Arc::new(ChecksumChunk {
            base: root.as_path().into(),
            rows: vec![row(1, "a")].into_boxed_slice(),
            retention: ChecksumRetention::new(()),
        })));
        update();
        assert_eq!(window().get_active_row(), 1, "a's row is in; b is next");
        assert!(window().get_selection_empty());
        // Click the finished row: the window is told at once, without a refresh.
        window().invoke_pick_row(0, false, false);
        assert!(!window().get_selection_empty());
        assert_eq!(window().get_cursor_row(), 0);
        assert_eq!(
            (window().get_copy_lines(), window().get_copy_lines_text()),
            (1, "1".into())
        );
        let rows = state.borrow().checksums.sessions[&id].rows().clone();
        // Read before `apply`, which borrows the presenter mutably.
        let generation = state.borrow().checksums.sessions[&id].generation;
        apply(ChecksumUpdate::Finished(std::sync::Arc::new(ChecksumRun {
            job: id,
            generation,
            algorithms,
            captured_roots: vec![root.clone()].into(),
            rows,
            summary: ChecksumSummary {
                completion: ChecksumCompletion::Cancelled,
                progress: ChecksumProgress::default(),
                diagnostics: vec![],
            },
            retention: ChecksumRetention::new(()),
        })));
        update();
        assert_eq!(window().get_active_row(), -1, "the run ended");
        assert!(!window().get_selection_empty(), "the selection stays");
    }

    /// A finished calculation of `count` files, refreshed into its window through this
    /// module's own refresh.
    struct Finished {
        h: crate::test_harness::Harness,
        state: Rc<RefCell<Presenter>>,
        send: Rc<dyn Fn(Command) -> bool>,
        id: ChecksumJobId,
        windows: Windows,
        sessions: RefCell<DialogSessions>,
        failures: Failures<ChecksumJobId>,
    }
    impl Finished {
        fn new(count: u64) -> Self {
            let h = crate::test_harness::Harness::with_location(
                UserPreferences::default(),
                std::env::temp_dir(),
            );
            let state = h.presenter.clone();
            let send: Rc<dyn Fn(Command) -> bool> = Rc::new(|_| true);
            state
                .borrow_mut()
                .request_checksums(PaneId::Left, send.as_ref());
            let id = *state.borrow().checksums.sessions.keys().next().unwrap();
            let algorithms = state.borrow().checksums.sessions[&id].algorithms;
            assert!(
                state
                    .borrow_mut()
                    .start_checksums(id, algorithms, false, send.as_ref())
            );
            let base: std::sync::Arc<std::path::Path> = std::env::temp_dir().into();
            let retention = ChecksumRetention::new(());
            let mut rows = ChecksumRows::default();
            let mut batch = Vec::new();
            for index in 0..count {
                batch.push(ChecksumRow {
                    id: ChecksumRowId(index + 1),
                    name: std::path::Path::new(&format!(
                        "album {:02}/IMG_{index:07}.jpg",
                        index % 40
                    ))
                    .into(),
                    identity: None,
                    kind: Some(EntryKind::File),
                    state: ChecksumRowState::Calculated,
                    bytes_read: 4096,
                    digests: ChecksumDigests::new(&[(
                        ChecksumAlgorithm::Sha256,
                        &[index as u8; 32],
                    )])
                    .unwrap(),
                    error: None,
                });
                if batch.len() == CHECKSUM_BATCH_SIZE || index + 1 == count {
                    rows.push(std::sync::Arc::new(ChecksumChunk {
                        base: base.clone(),
                        rows: std::mem::take(&mut batch).into_boxed_slice(),
                        retention: retention.clone(),
                    }));
                }
            }
            let generation = state.borrow().checksums.sessions[&id].generation;
            state
                .borrow_mut()
                .apply(Event::Checksum(Box::new(ChecksumEvent {
                    job: id,
                    generation,
                    update: ChecksumUpdate::Finished(std::sync::Arc::new(ChecksumRun {
                        job: id,
                        generation,
                        algorithms,
                        captured_roots: vec![std::env::temp_dir()].into(),
                        rows,
                        summary: ChecksumSummary {
                            completion: ChecksumCompletion::Completed,
                            progress: ChecksumProgress {
                                calculated: count,
                                ..Default::default()
                            },
                            diagnostics: vec![],
                        },
                        retention,
                    })),
                })));
            let closing = Rc::new(Cell::new(false));
            let failures = Failures::new(
                &h.window,
                &state,
                &send,
                &closing,
                "Checksums",
                Presenter::close_checksums,
            );
            let finished = Self {
                h,
                state,
                send,
                id,
                windows: Rc::default(),
                sessions: RefCell::new(DialogSessions::default()),
                failures,
            };
            finished.refresh();
            finished
        }
        fn refresh(&self) {
            refresh(
                &self.h.window,
                &self.windows,
                &self.sessions,
                &self.state,
                &self.send,
                &self.failures,
            );
        }
        fn window(&self) -> ChecksumWindow {
            self.windows.borrow()[&self.id].window.clone_strong()
        }
    }

    /// Counts what the rows model tells its views.
    #[derive(Default)]
    struct Notifications {
        resets: Cell<usize>,
        changed: Cell<usize>,
    }
    impl i_slint_core::model::ModelChangeListener for Notifications {
        fn row_changed(self: std::pin::Pin<&Self>, _: usize) {
            self.changed.set(self.changed.get() + 1);
        }
        fn row_added(self: std::pin::Pin<&Self>, _: usize, _: usize) {}
        fn row_removed(self: std::pin::Pin<&Self>, _: usize, _: usize) {}
        fn reset(self: std::pin::Pin<&Self>) {
            self.resets.set(self.resets.get() + 1);
        }
    }

    /// Select all, Deselect all and a click that drops a large selection tell the list
    /// once, by a reset, however many rows they mark; a click or a key that marks a few
    /// rows tells only those rows. The marks read back are the selection's.
    #[test]
    fn bulk_mark_changes_notify_the_list_once_and_small_ones_per_row() {
        use i_slint_core::model::ModelChangeListenerContainer;
        let count = 20_000;
        let f = Finished::new(count);
        let window = f.window();
        let listener = Box::pin(ModelChangeListenerContainer::new(Notifications::default()));
        window
            .get_rows()
            .model_tracker()
            .attach_peer(listener.as_ref().model_peer());
        let notified = || {
            let counts = (listener.resets.get(), listener.changed.get());
            listener.resets.set(0);
            listener.changed.set(0);
            counts
        };
        let selected = |row: usize| window.get_rows().row_data(row).unwrap().selected;
        window.invoke_select_all();
        assert_eq!(notified(), (1, 0), "Ctrl+A over {count} rows");
        assert!(selected(0) && selected(count as usize - 1));
        // A click drops every other mark: one reset again.
        window.invoke_pick_row(5, false, false);
        assert_eq!(notified(), (1, 0), "a click that drops {count} marks");
        assert!(selected(5) && !selected(4) && !selected(6));
        // A few rows: each one alone.
        window.invoke_pick_row(7, true, false);
        let (resets, changed) = notified();
        assert_eq!(resets, 0);
        assert!(
            (1..=4).contains(&changed),
            "{changed} rows told of a Ctrl+click"
        );
        window.invoke_move_cursor(8, false, false);
        let (resets, changed) = notified();
        assert_eq!(resets, 0);
        assert!(
            (1..=4).contains(&changed),
            "{changed} rows told of a key move"
        );
        assert!(selected(8) && !selected(5) && !selected(7));
        window.invoke_deselect_all();
        let (resets, changed) = notified();
        assert!(
            resets + changed <= 1,
            "Ctrl+D: {resets} resets, {changed} rows"
        );
        window.invoke_select_all();
        window.invoke_deselect_all();
        assert_eq!(notified(), (2, 0), "Ctrl+A, Ctrl+D");
        assert!((0..count as usize).step_by(997).all(|row| !selected(row)));
    }

    /// What the UI thread spends on a selection or a copy at `MIDNIGHT_CHECKSUM_ROWS` rows
    /// (100,000 by default), through the window's own callbacks: the presenter, the model
    /// notifications and the published properties. Reports timings without thresholds;
    /// run alone, in release, with --ignored --nocapture.
    #[test]
    #[ignore = "explicit checksum selection benchmark; reports timings without pass thresholds"]
    fn checksum_selection_benchmark() {
        use i_slint_core::model::ModelChangeListenerContainer;
        use std::time::Instant;
        let count: u64 = std::env::var("MIDNIGHT_CHECKSUM_ROWS")
            .ok()
            .and_then(|rows| rows.parse().ok())
            .unwrap_or(100_000);
        let f = Finished::new(count);
        let (h, state, send, id, windows, sessions, failures) = (
            &f.h,
            &f.state,
            &f.send,
            f.id,
            &f.windows,
            &f.sessions,
            &f.failures,
        );
        let listener = Box::pin(ModelChangeListenerContainer::new(Notifications::default()));
        f.window()
            .get_rows()
            .model_tracker()
            .attach_peer(listener.as_ref().model_peer());
        f.window().invoke_select_all();
        f.window().invoke_deselect_all();
        println!(
            "Ctrl+A then Ctrl+D at {count} rows: {} resets, {} row notifications",
            listener.resets.get(),
            listener.changed.get()
        );
        let window = windows.borrow()[&id].window.clone_strong();
        let time = |what: &str, action: &dyn Fn()| {
            let mut samples: Vec<f64> = (0..5)
                .map(|_| {
                    let start = Instant::now();
                    action();
                    start.elapsed().as_secs_f64() * 1000.0
                })
                .collect();
            samples.sort_by(f64::total_cmp);
            println!(
                "{what} at {count} rows: median {:.2} ms, worst {:.2} ms",
                samples[2], samples[4]
            );
        };
        time("select all (Ctrl+A)", &|| window.invoke_select_all());
        // The two ways of telling the list, alone: every row (before 24 November) or one
        // reset (`MARKS_RESET`).
        let rows = windows.borrow()[&id].rows.clone();
        let every = 0..rows.row_count();
        time("notify every row (the old Select all)", &|| {
            every.clone().for_each(|row| rows.notify.row_changed(row))
        });
        time("notify one reset", &|| rows.notify.reset());
        time("click after select all (clears every mark)", &|| {
            window.invoke_select_all();
            window.invoke_pick_row(5, false, false);
        });
        let far = i32::try_from(count * 6 / 10).unwrap_or(i32::MAX);
        time("Shift+click range over 60% of the rows", &|| {
            window.invoke_pick_row(10, false, false);
            window.invoke_pick_row(far, false, true);
        });
        time("Ctrl+D after select all", &|| {
            window.invoke_select_all();
            window.invoke_deselect_all();
        });
        window.invoke_select_all();
        let copy = |kind: &str, copy: &dyn Fn(&mut Presenter)| {
            time(kind, &|| copy(&mut state.borrow_mut()));
        };
        copy("copy as checksum file, every row selected", &|p| {
            p.copy_checksum_file(id, ChecksumAlgorithm::Sha256, &|_| true)
        });
        copy("Ctrl+C checksums, every row selected", &|p| {
            p.copy_checksums(id, ChecksumAlgorithm::Sha256, false, &|_| true)
        });
        // What the clipboard worker then does, off the UI thread.
        {
            use midnight_core::{
                ChecksumCopyKind, ChecksumCopyRows, ChecksumCopyText, GeneratedTextSource,
            };
            let rows = state.borrow().checksums.sessions[&id].rows().clone();
            let bits: std::sync::Arc<[u64]> = state.borrow().checksums.sessions[&id]
                .selection
                .words()
                .into();
            for (what, kind) in [
                (
                    "checksum-file lines",
                    ChecksumCopyKind::Lines(ChecksumAlgorithm::Sha256),
                ),
                (
                    "checksums",
                    ChecksumCopyKind::Hashes(ChecksumAlgorithm::Sha256),
                ),
            ] {
                time(
                    &format!("clipboard worker builds the {what}, every row selected"),
                    &|| {
                        let copy = ChecksumCopyText::new(
                            rows.clone(),
                            ChecksumCopyRows::Selected(bits.clone()),
                            kind,
                            std::env::temp_dir(),
                        );
                        copy.prepare(&midnight_core::Cancellation::default())
                            .unwrap();
                        drop(copy.take_text());
                    },
                );
            }
        }
        time(
            "Ctrl+A then a key move (cursor and marks published)",
            &|| {
                window.invoke_select_all();
                window.invoke_move_cursor(7, false, false);
            },
        );
        // A run's batch changes the session: the window is refreshed whole, the selection's
        // counts included.
        window.invoke_select_all();
        time("window refresh after a change, every row selected", &|| {
            if let Some(s) = state.borrow_mut().checksums.sessions.get_mut(&id) {
                s.revision = s.revision.wrapping_add(1);
            }
            refresh(&h.window, windows, sessions, state, send, failures);
        });
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
