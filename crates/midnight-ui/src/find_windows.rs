//! Native modeless Find windows. No filesystem access or content decoding here.
use crate::dialog_host::{CloseDisposition, DialogHost, DialogOwner, DialogSessions};
use crate::tool_window_failure::Failures;
use crate::{
    FindDraft, FindRow, FindWindow, MainWindow, Presenter, Theme, presenter::find::FindForm,
    shown_text,
};
use midnight_core::*;
use slint::{ComponentHandle, Model, ModelNotify, ModelTracker, SharedString, VecModel};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::{Rc, Weak},
    sync::Arc,
};
#[cfg(test)]
#[path = "find_window_order_tests.rs"]
mod order_tests;

struct Rows {
    state: Weak<RefCell<Presenter>>,
    session: FindSessionId,
    notify: ModelNotify,
    structure: Cell<(u64, usize)>,
    view: RefCell<Option<Arc<FindOrder>>>,
}
impl Model for Rows {
    type Data = FindRow;
    fn row_count(&self) -> usize {
        self.state.upgrade().map_or(0, |state| {
            state
                .borrow()
                .finds
                .sessions
                .get(&self.session)
                .map_or(0, |s| s.results.len())
        })
    }
    fn row_data(&self, row: usize) -> Option<FindRow> {
        let state = self.state.upgrade()?;
        let p = state.borrow();
        let session = p.finds.sessions.get(&self.session)?;
        let id = self.id_for_row(row)?;
        let result = session.results.get(id.0.checked_sub(1)? as usize)?;
        let entry = &result.entry;
        Some(FindRow {
            token: format!("{}:{}", session.generation.0, result.id.0).into(),
            name: shown_text(
                &entry
                    .path
                    .file_name()
                    .unwrap_or(entry.path.as_os_str())
                    .to_string_lossy(),
            ),
            extension: entry
                .path
                .extension()
                .map_or_else(SharedString::default, |value| {
                    shown_text(&value.to_string_lossy())
                }),
            path: shown_text(&entry.path.parent().unwrap_or(&entry.path).to_string_lossy()),
            size: if entry.kind == EntryKind::Directory {
                "<DIR>".into()
            } else {
                entry.metadata.size.map_or_else(
                    || "Unknown size".into(),
                    |n| format!("{} B", p.number_locale.format_u64(n)).into(),
                )
            },
            modified: crate::time_presentation::modified(&entry.metadata)
                .map_or_else(SharedString::default, |d| {
                    format!("{} {}", d.date, d.time).into()
                }),
            directory: entry.kind == EntryKind::Directory,
            duplicate_group: result
                .duplicate_group
                .map_or_else(SharedString::default, |group| {
                    format!("Group {group} · {} files", result.duplicate_group_size).into()
                }),
            group_start: result.duplicate_group.is_some()
                && row
                    .checked_sub(1)
                    .and_then(|previous| self.id_for_row(previous))
                    .and_then(|id| session.results.get(id.0.checked_sub(1)? as usize))
                    .is_none_or(|previous| previous.duplicate_group != result.duplicate_group),
            group_shade: result.duplicate_group.is_some_and(|group| group % 2 == 0),
        })
    }
    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }
}
impl Rows {
    fn id_for_row(&self, row: usize) -> Option<FindResultId> {
        if row >= self.structure.get().1 {
            return None;
        }
        Some(
            self.view
                .borrow()
                .as_ref()
                .and_then(|order| order.ids.get(row))
                .copied()
                .unwrap_or(FindResultId(row as u64 + 1)),
        )
    }
    fn row_for_id(&self, id: FindResultId) -> Option<usize> {
        let index = usize::try_from(id.0.checked_sub(1)?).ok()?;
        let row = self
            .view
            .borrow()
            .as_ref()
            .and_then(|order| order.positions.get(index))
            .copied()
            .unwrap_or(index);
        (row < self.structure.get().1).then_some(row)
    }
    fn update(
        &self,
        generation: u64,
        count: usize,
        view: Option<Arc<FindOrder>>,
    ) -> Option<Arc<FindOrder>> {
        let old = self.structure.replace((generation, count));
        let changed = match (&*self.view.borrow(), &view) {
            (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
            (None, None) => false,
            _ => true,
        };
        let retired = if changed {
            self.view.replace(view)
        } else {
            None
        };
        if old.0 != generation || old.1 > count || changed {
            self.notify.reset();
        } else if old.1 < count {
            self.notify.row_added(old.1, count - old.1);
        }
        retired
    }
}
struct WindowRecord {
    window: FindWindow,
    host: Rc<DialogHost>,
    shown: bool,
    rows: Rc<Rows>,
    roots: Vec<std::path::PathBuf>,
    revision: Option<u64>,
}
type Windows = Rc<RefCell<HashMap<FindSessionId, WindowRecord>>>;

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
        "Find files",
        Presenter::close_find,
    );
    {
        let weak = main.as_weak();
        let state = state.clone();
        let closing = closing.clone();
        main.on_find_files(move |pane| {
            if closing.get() {
                return;
            }
            state.borrow_mut().create_find(crate::pane(pane));
            if let Some(main) = weak.upgrade() {
                crate::refresh(&main, &state.borrow());
            }
        });
    }
    {
        let weak = main.as_weak();
        let windows = windows.clone();
        let state = state.clone();
        let send = send.clone();
        let closing = closing.clone();
        let failures = failures.clone();
        main.on_refresh_find_windows(move || {
            if closing.get() {
                return;
            }
            let Some(main) = weak.upgrade() else {
                return;
            };
            refresh(&main, &windows, &sessions, &state, &send, &failures);
        });
    }
    main.on_close_find_windows(move || {
        closing.set(true);
        let records = std::mem::take(&mut *windows.borrow_mut());
        for (id, record) in records {
            // No Presenter borrow: the host's exit timer may already own it.
            // The engine Shutdown command subsequently cancels every session even
            // if this bounded per-window close cannot enter the full queue.
            let _ = send(Command::CloseFind(id));
            record.host.close(record.window.window());
        }
    });
}

fn refresh(
    main: &MainWindow,
    windows: &Windows,
    sessions: &RefCell<DialogSessions>,
    state: &Rc<RefCell<Presenter>>,
    send: &Rc<dyn Fn(Command) -> bool>,
    failures: &Failures<FindSessionId>,
) {
    let ids: Vec<_> = state.borrow().finds.sessions.keys().copied().collect();
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
        let Some(s) = p.finds.sessions.get(&id) else {
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
        let view_keys = p.shortcut_hint("files.view");
        let basic_keys = p.shortcut_hint("files.view-basic");
        let edit_keys = p.shortcut_hint("files.edit");
        w.set_view_hint(
            format!(
                "View internally{}; plain text{}.",
                shortcut_suffix(view_keys),
                shortcut_suffix(basic_keys)
            )
            .into(),
        );
        w.set_edit_hint(
            format!(
                "Edit with the configured external editor{}.",
                shortcut_suffix(edit_keys)
            )
            .into(),
        );
        if record.revision == Some(s.revision) {
            crate::tooltip::synchronize_find(w);
            continue;
        }
        record.revision = Some(s.revision);
        w.set_running(s.running);
        w.set_stopping(s.stopping);
        w.set_duplicate_results(s.duplicates);
        // Copy details formats the session again, with raw paths.
        w.set_error(shown_text(&s.error));
        w.set_attention(
            !s.error.is_empty()
                || s.summary.as_ref().is_some_and(|summary| {
                    matches!(
                        summary.completion,
                        FindCompletion::Incomplete | FindCompletion::Failed
                    )
                }),
        );
        w.set_progress(progress(s, &p.number_locale).into());
        w.set_current_directory(s.current_directory.as_ref().map_or_else(
            SharedString::default,
            |path| {
                let mut text = match s.progress.phase {
                    FindPhase::Scanning => "Scanning: ",
                    FindPhase::HashingDuplicates => "Hashing: ",
                    FindPhase::ComparingDuplicates => "Comparing: ",
                }
                .to_owned();
                crate::error_presentation::append_path(&mut text, path);
                shown_text(&text)
            },
        ));
        w.set_sort_column(sort_index(s.sort.column));
        w.set_sort_descending(s.sort.descending);
        w.set_details(shown_text(&details(s, &p.number_locale)));
        if record.roots != s.roots {
            record.roots = s.roots.clone();
            w.set_roots(
                Rc::new(VecModel::from(
                    s.roots
                        .iter()
                        .map(|p| shown_text(&p.to_string_lossy()))
                        .collect::<Vec<_>>(),
                ))
                .into(),
            );
        }
        let selected = (record.rows.structure.get().0 == s.generation.0)
            .then(|| {
                usize::try_from(w.get_selected_row())
                    .ok()
                    .and_then(|row| record.rows.id_for_row(row))
            })
            .flatten();
        let retired = record
            .rows
            .update(s.generation.0, s.results.len(), s.order.clone());
        w.set_selected_row(
            selected
                .and_then(|id| record.rows.row_for_id(id))
                .and_then(|row| i32::try_from(row).ok())
                .unwrap_or(-1),
        );
        crate::tooltip::synchronize_find(w);
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
        if let Some(retired) = retired {
            state.borrow().retire_find_order(retired);
        }
        if let Err(error) = shown {
            windows.borrow_mut().remove(&id);
            failures.record(id, error.to_string());
        }
    }
}

fn create(
    main: &MainWindow,
    id: FindSessionId,
    sessions: &RefCell<DialogSessions>,
    state: &Rc<RefCell<Presenter>>,
    send: &Rc<dyn Fn(Command) -> bool>,
) -> Result<WindowRecord, slint::PlatformError> {
    let session = sessions
        .borrow_mut()
        .allocate()
        .map_err(slint::PlatformError::from)?;
    let (window, host) = DialogHost::create_tool(session, DialogOwner::new(main), FindWindow::new)?;
    let host = Rc::new(host);
    crate::tooltip::install_find(&window)?;
    let rows = Rc::new(Rows {
        state: Rc::downgrade(state),
        session: id,
        notify: ModelNotify::default(),
        structure: Cell::new((0, 0)),
        view: RefCell::new(None),
    });
    window.set_rows(rows.clone().into());
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
    callback!(on_search, |p, send, draft| {
        p.start_find(id, form(draft), send);
    });
    callback!(on_stop, |p, send| {
        p.stop_find(id, send);
    });
    callback!(on_sort, |p, send, column| {
        if let Some(column) = sort_column(column) {
            p.sort_find(id, column, send);
        }
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
                p.close_find(id, send.as_ref());
                if p.finds.sessions.contains_key(&id) {
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
    callback!(on_add_root, |p, _send, text| {
        p.add_find_root(id, &text);
    });
    callback!(on_remove_root, |p, _send, row| {
        p.remove_find_root(id, row);
    });
    callback!(on_reveal, |p, send, row| {
        p.reveal_find(id, &row, send);
    });
    callback!(on_file_action, |p, send, row, action| {
        if let Some(action) = file_action(action) {
            p.find_file_action(id, &row, action, send);
        }
    });
    {
        let host = host.clone();
        let state = state.clone();
        let send = send.clone();
        let main = main.as_weak();
        window.on_action_key(move |event, token| {
            if !host.is_current(session) {
                return false;
            }
            let Some(chord) = crate::shortcut_input::chord(&event) else {
                return false;
            };
            let action = state.borrow().find_action_key(&chord);
            let Some(action) = action else {
                return false;
            };
            state
                .borrow_mut()
                .find_file_action(id, &token, action, send.as_ref());
            if let Some(main) = main.upgrade() {
                crate::refresh(&main, &state.borrow());
            }
            true
        });
    }
    callback!(on_copy_details, |p, send| {
        if let Some(s) = p.finds.sessions.get(&id) {
            let locale = &p.number_locale;
            let text = format!(
                "{}\n\n{}\n{}",
                progress(s, locale),
                s.error,
                details(s, locale)
            );
            p.copy_viewer_text(text, send);
        }
    });
    let weak = window.as_weak();
    window.window().on_close_requested(move || {
        if let Some(w) = weak.upgrade() {
            w.invoke_close_session();
        }
        slint::CloseRequestResponse::KeepWindowShown
    });
    Ok(WindowRecord {
        window,
        host,
        shown: false,
        rows,
        roots: vec![],
        revision: None,
    })
}

fn form(draft: FindDraft) -> FindForm {
    FindForm {
        masks: draft.masks.to_string(),
        name_mode: match draft.mode {
            1 => FilterMode::Regex,
            2 => FilterMode::Expression,
            _ => FilterMode::Masks,
        },
        recursive: draft.recursive,
        name_case: draft.name_case,
        content: draft.content_enabled.then(|| FindContentQuery {
            text: draft.content.to_string(),
            case_sensitive: draft.content_case,
            whole_words: draft.whole_words,
        }),
        modified_within: draft
            .modified_enabled
            .then(|| std::time::Duration::from_secs(draft.modified_seconds.max(0) as u64)),
        duplicates: draft.duplicates,
    }
}

fn sort_column(index: i32) -> Option<FindSortColumn> {
    Some(match index {
        0 => FindSortColumn::Name,
        1 => FindSortColumn::Extension,
        2 => FindSortColumn::Path,
        3 => FindSortColumn::Size,
        4 => FindSortColumn::Modified,
        _ => return None,
    })
}
fn sort_index(column: FindSortColumn) -> i32 {
    match column {
        FindSortColumn::Name => 0,
        FindSortColumn::Extension => 1,
        FindSortColumn::Path => 2,
        FindSortColumn::Size => 3,
        FindSortColumn::Modified => 4,
    }
}

fn progress(s: &crate::presenter::find::FindSession, locale: &NumberLocale) -> String {
    let status = if !s.error.is_empty() {
        "Error · see Details"
    } else if s.stopping {
        "Stopping…"
    } else if s.running {
        match s.progress.phase {
            FindPhase::Scanning => "Searching…",
            FindPhase::HashingDuplicates => "Hashing candidates…",
            FindPhase::ComparingDuplicates => "Comparing contents…",
        }
    } else {
        match s.summary.as_ref().map(|s| s.completion) {
            Some(FindCompletion::Completed) => "Completed",
            Some(FindCompletion::Incomplete) => "Incomplete · see Details",
            Some(FindCompletion::Cancelled) => "Cancelled",
            Some(FindCompletion::Failed) => "Failed · see Details",
            None => "Ready",
        }
    };
    let mut text = format!(
        "{status} · {} found · {} examined",
        locale.format_u64(s.results.len() as u64),
        locale.format_u64(s.progress.examined)
    );
    if s.duplicates {
        text.push_str(&format!(
            " · {} groups · {} candidates",
            locale.format_u64(s.progress.duplicate_groups),
            locale.format_u64(s.progress.duplicate_candidates)
        ));
    }
    text
}

fn details(s: &crate::presenter::find::FindSession, locale: &NumberLocale) -> String {
    let p = &s.progress;
    let count = |value: u64| locale.format_u64(value);
    let mut text = format!(
        "{} files · {} folders · {} content bytes\nSkipped: {} links, {} special entries, {} mount boundaries\n{} errors · {} size-limited files",
        count(p.files),
        count(p.directories),
        count(p.content_bytes),
        count(p.skipped_links),
        count(p.skipped_special),
        count(p.skipped_mounts),
        count(p.errors),
        count(p.limited_files)
    );
    if s.duplicates {
        text.push_str(&format!(
            "\nIdentical files: {} verified groups · {} candidates · {} bytes read to check duplicates",
            count(p.duplicate_groups),
            count(p.duplicate_candidates),
            count(p.duplicate_bytes)
        ));
    }
    if let Some(summary) = &s.summary {
        if !summary.reached_budgets.is_empty() {
            let limits: Vec<_> = summary
                .reached_budgets
                .iter()
                .copied()
                .map(budget)
                .collect();
            text.push_str(&format!("\nReached limits: {}", limits.join(", ")));
        }
        for error in &summary.diagnostics {
            text.push_str(&format!(
                "\n\n{}",
                crate::error_presentation::describe(error)
            ));
        }
    }
    text
}

/// User wording for a backend resource budget, never a Rust identifier.
fn budget(budget: FindBudget) -> &'static str {
    match budget {
        FindBudget::Results => "result count",
        FindBudget::Nodes => "visited entries",
        FindBudget::Depth => "folder depth",
        FindBudget::RetainedBytes => "retained memory",
        FindBudget::FileContent => "content read per file",
        FindBudget::TotalContent => "total content read",
        FindBudget::DuplicateCandidates => "duplicate candidates",
    }
}

fn file_action(action: i32) -> Option<crate::presenter::find::FileAction> {
    use crate::presenter::find::FileAction;
    Some(match action {
        0 => FileAction::View(false),
        1 => FileAction::View(true),
        2 => FileAction::Edit,
        3 => FileAction::Open,
        _ => return None,
    })
}
fn shortcut_suffix(keys: &str) -> String {
    if keys.is_empty() {
        String::new()
    } else {
        format!(" ({keys})")
    }
}
