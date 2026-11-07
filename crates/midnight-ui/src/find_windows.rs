//! Native modeless Find windows. No filesystem access or content decoding here.
use crate::tool_window_failure::Failures;
use crate::{
    FindDraft, FindRow, FindWindow, MainWindow, Presenter, Theme, presenter::find::FindForm,
};
use midnight_core::*;
use slint::{ComponentHandle, Model, ModelNotify, ModelTracker, SharedString, VecModel};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::{Rc, Weak},
};

struct Rows {
    state: Weak<RefCell<Presenter>>,
    session: FindSessionId,
    notify: ModelNotify,
    structure: Cell<(u64, usize)>,
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
        let result = session.results.get(row)?;
        let entry = &result.entry;
        Some(FindRow {
            token: format!("{}:{}", session.generation.0, result.id.0).into(),
            name: entry
                .path
                .file_name()
                .unwrap_or(entry.path.as_os_str())
                .to_string_lossy()
                .as_ref()
                .into(),
            path: entry
                .path
                .parent()
                .unwrap_or(&entry.path)
                .to_string_lossy()
                .as_ref()
                .into(),
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
struct WindowRecord {
    window: FindWindow,
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
            refresh(&main, &windows, &state, &send, &failures);
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
            let _ = record.window.hide();
        }
    });
}

fn refresh(
    main: &MainWindow,
    windows: &Windows,
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
        let Some(s) = p.finds.sessions.get(&id) else {
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
        if record.revision == Some(s.revision) {
            crate::tooltip::synchronize_find(w);
            continue;
        }
        record.revision = Some(s.revision);
        w.set_running(s.running);
        w.set_stopping(s.stopping);
        w.set_error(s.error.clone().into());
        w.set_attention(
            !s.error.is_empty()
                || s.summary.as_ref().is_some_and(|summary| {
                    matches!(
                        summary.completion,
                        FindCompletion::Incomplete | FindCompletion::Failed
                    )
                }),
        );
        w.set_progress(progress(s).into());
        w.set_details(details(s).into());
        if record.roots != s.roots {
            record.roots = s.roots.clone();
            w.set_roots(
                Rc::new(VecModel::from(
                    s.roots
                        .iter()
                        .map(|p| SharedString::from(p.to_string_lossy().as_ref()))
                        .collect::<Vec<_>>(),
                ))
                .into(),
            );
        }
        if record.rows.structure.get().0 != s.generation.0 {
            w.set_selected_row(-1);
        }
        record.rows.update(s.generation.0, s.results.len());
        crate::tooltip::synchronize_find(w);
    }
}

fn create(
    main: &MainWindow,
    id: FindSessionId,
    state: &Rc<RefCell<Presenter>>,
    send: &Rc<dyn Fn(Command) -> bool>,
) -> Result<WindowRecord, slint::PlatformError> {
    let window = FindWindow::new()?;
    crate::tooltip::install_find(&window)?;
    let rows = Rc::new(Rows {
        state: Rc::downgrade(state),
        session: id,
        notify: ModelNotify::default(),
        structure: Cell::new((0, 0)),
    });
    window.set_rows(rows.clone().into());
    macro_rules! callback {
        ($name:ident, |$p:ident, $send:ident $(, $arg:ident)*| $body:block) => {{
            let state = state.clone(); let sender = send.clone(); let weak = main.as_weak();
            window.$name(move |$($arg),*| {
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
    callback!(on_close_session, |p, send| {
        p.close_find(id, send);
    });
    callback!(on_add_root, |p, _send, text| {
        p.add_find_root(id, &text);
    });
    callback!(on_remove_root, |p, _send, row| {
        p.remove_find_root(id, row);
    });
    callback!(on_reveal, |p, send, row| {
        p.reveal_find(id, &row, send);
    });
    callback!(on_copy_details, |p, send| {
        if let Some(s) = p.finds.sessions.get(&id) {
            let text = format!("{}\n\n{}\n{}", progress(s), s.error, details(s));
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
    window.show()?;
    window.invoke_begin();
    Ok(WindowRecord {
        window,
        rows,
        roots: vec![],
        revision: None,
    })
}

fn form(draft: FindDraft) -> FindForm {
    FindForm {
        masks: draft.masks.to_string(),
        recursive: draft.recursive,
        name_case: draft.name_case,
        content: draft.content_enabled.then(|| FindContentQuery {
            text: draft.content.to_string(),
            case_sensitive: draft.content_case,
        }),
    }
}

fn progress(s: &crate::presenter::find::FindSession) -> String {
    let status = if !s.error.is_empty() {
        "Error · see Details"
    } else if s.stopping {
        "Stopping…"
    } else if s.running {
        "Searching…"
    } else {
        match s.summary.as_ref().map(|s| s.completion) {
            Some(FindCompletion::Completed) => "Completed",
            Some(FindCompletion::Incomplete) => "Incomplete · see Details",
            Some(FindCompletion::Cancelled) => "Cancelled",
            Some(FindCompletion::Failed) => "Failed · see Details",
            None => "Ready",
        }
    };
    format!(
        "{status} · {} found · {} examined",
        s.results.len(),
        s.progress.examined
    )
}

fn details(s: &crate::presenter::find::FindSession) -> String {
    let p = &s.progress;
    let mut text = format!(
        "{} files · {} folders · {} content bytes\nSkipped: {} links, {} special entries, {} mount boundaries\n{} errors · {} size-limited files",
        p.files,
        p.directories,
        p.content_bytes,
        p.skipped_links,
        p.skipped_special,
        p.skipped_mounts,
        p.errors,
        p.limited_files
    );
    if let Some(summary) = &s.summary {
        if !summary.reached_budgets.is_empty() {
            text.push_str(&format!("\nReached limits: {:?}", summary.reached_budgets));
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
