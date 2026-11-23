//! Output windows of Run directly commands own presentation only. Each copies its
//! session's new lines when the presenter says the session changed; the program's
//! pipes are read by backend threads and never wait for this window.
use crate::{
    CommandOutputHub, CommandOutputRow, CommandOutputWindow, MainWindow, Presenter,
    dialog_host::{CloseDisposition, DialogHost, DialogOwner, DialogSession, DialogSessions},
    dialog_presentation::{self, DialogThemeTarget},
    tool_window_failure::Failures,
};
use midnight_core::*;
use slint::{ComponentHandle, Model, ModelNotify, ModelRc, ModelTracker, SharedString};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    rc::Rc,
    time::Duration,
};

#[cfg(test)]
#[path = "command_output_windows_tests.rs"]
mod tests;

type ThemeRevision = (i32, i32, String, i32, i32);

impl DialogThemeTarget for CommandOutputWindow {
    fn apply_theme(
        &self,
        scheme: i32,
        density: i32,
        family: SharedString,
        desktop: i32,
        touch: i32,
    ) {
        self.invoke_apply_preferences(scheme, density, family, desktop, touch);
    }
}

/// A window's rows: the session's retained lines followed by the lines still being
/// written. Lines are only added at the end and dropped at the front, as the session
/// does, so an update notifies just those rows.
#[derive(Default)]
pub(crate) struct Rows {
    lines: RefCell<VecDeque<CommandOutputRow>>,
    partial: RefCell<Vec<CommandOutputRow>>,
    notify: ModelNotify,
    /// Sequence number of the first line and after the last one.
    first: Cell<u64>,
    next: Cell<u64>,
    /// The longest line, in characters, since the last Clear.
    columns: Cell<usize>,
}

/// How many fixed-width columns `text` takes: East Asian wide characters and emoji take
/// two. An estimate for the list's scrollable width, not a text layout.
pub(crate) fn columns(text: &str) -> usize {
    text.chars()
        .map(|character| match u32::from(character) {
            0x1100..=0x115F
            | 0x2E80..=0xA4CF
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE30..=0xFE4F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x1F300..=0x1F64F
            | 0x1F900..=0x1F9FF
            | 0x20000..=0x3FFFD => 2,
            _ => 1,
        })
        .sum()
}

fn row(line: &OutputLine, partial: bool) -> CommandOutputRow {
    CommandOutputRow {
        text: crate::shown_text(&line.text),
        kind: match line.stream {
            OutputStream::Stdout => 0,
            OutputStream::Stderr => 1,
            OutputStream::Notice => 2,
        },
        partial,
    }
}

impl Rows {
    /// Bring the rows to `view`, read with `self.next()`. No borrow is held while the
    /// views are notified.
    pub(crate) fn apply(&self, view: &OutputView) {
        let (gone, kept) = {
            let mut lines = self.lines.borrow_mut();
            let gone = (view.first.saturating_sub(self.first.get()) as usize).min(lines.len());
            lines.drain(..gone);
            (gone, lines.len())
        };
        if gone > 0 {
            self.notify.row_removed(0, gone);
        }
        if view.first > self.first.get() {
            self.first.set(view.first);
            if kept == 0 {
                self.columns.set(0);
            }
        }
        let old_partial = std::mem::take(&mut *self.partial.borrow_mut()).len();
        if old_partial > 0 {
            self.notify.row_removed(kept, old_partial);
        }
        let mut columns = self.columns.get();
        for line in view.lines.iter().chain(&view.partial) {
            columns = columns.max(self::columns(&line.text));
        }
        self.columns.set(columns);
        self.lines
            .borrow_mut()
            .extend(view.lines.iter().map(|line| row(line, false)));
        if !view.lines.is_empty() {
            self.notify.row_added(kept, view.lines.len());
        }
        self.next.set(view.next.max(self.first.get()));
        *self.partial.borrow_mut() = view.partial.iter().map(|line| row(line, true)).collect();
        if !view.partial.is_empty() {
            self.notify
                .row_added(kept + view.lines.len(), view.partial.len());
        }
    }

    pub(crate) fn next(&self) -> u64 {
        self.next.get()
    }

    pub(crate) fn columns(&self) -> usize {
        self.columns.get()
    }

    /// The next row from `from` (exclusive) on, or before it when `backward`, whose text
    /// contains `needle` ignoring case; wraps around.
    pub(crate) fn find(&self, needle: &str, from: i32, backward: bool) -> Option<usize> {
        let needle = needle.to_lowercase();
        if needle.is_empty() {
            return None;
        }
        let count = self.row_count();
        let start = usize::try_from(from).ok();
        (1..=count)
            .map(|step| match (start, backward) {
                (Some(start), false) => (start + step) % count,
                (Some(start), true) => (start + count - step % count) % count,
                (None, false) => step - 1,
                (None, true) => count - step,
            })
            .find(|&index| {
                self.row_data(index)
                    .is_some_and(|row| row.text.to_lowercase().contains(&needle))
            })
    }
}

impl Model for Rows {
    type Data = CommandOutputRow;
    fn row_count(&self) -> usize {
        self.lines.borrow().len() + self.partial.borrow().len()
    }
    fn row_data(&self, row: usize) -> Option<CommandOutputRow> {
        let lines = self.lines.borrow();
        match lines.get(row) {
            Some(line) => Some(line.clone()),
            None => self.partial.borrow().get(row - lines.len()).cloned(),
        }
    }
    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }
}

/// What the window's first line says, and whether it reports a failure.
pub(crate) fn status(view: &OutputView, locale: &NumberLocale) -> (String, bool) {
    let Some(finish) = &view.finished else {
        if view.stopping {
            return ("Stopping…".into(), false);
        }
        let run = view.runs.last().map_or(0, |run| run.index + 1);
        return if view.planned > 1 {
            (
                format!(
                    "Running {} of {}…",
                    locale.format_u64(run.max(1)),
                    locale.format_u64(view.planned)
                ),
                false,
            )
        } else {
            ("Running…".into(), false)
        };
    };
    let failed_runs = view
        .runs
        .iter()
        .filter(|run| !run.exit.is_some_and(ProgramExit::success))
        .count();
    let stopped = view.runs.iter().any(|run| run.stopped) || finish.cancelled;
    let total: Duration = view.runs.iter().filter_map(|run| run.duration).sum();
    let failed = failed_runs > 0 || finish.error.is_some() || stopped || view.runs.is_empty();
    let text = match (view.runs.as_slice(), view.planned) {
        ([], _) => match &finish.error {
            Some(error) => error.clone(),
            None if finish.cancelled => "Stopped before the program started".into(),
            None => "The program did not start".into(),
        },
        ([run], 1) => {
            let exit = run
                .exit
                .map_or_else(|| "Exit status unknown".to_owned(), ProgramExit::describe);
            let lead = if run.stopped {
                "Stopped"
            } else if run.exit.is_some_and(ProgramExit::success) {
                "Finished"
            } else {
                "Failed"
            };
            format!("{lead} · {exit} · {}", describe_duration(total))
        }
        (runs, planned) => {
            let lead = if stopped { "Stopped" } else { "Finished" };
            let ran = format!(
                "{} of {} runs",
                locale.format_u64(runs.len() as u64),
                locale.format_u64(planned)
            );
            let outcome = if failed_runs == 0 {
                "all exited with code 0".to_owned()
            } else {
                format!("{} failed", locale.format_u64(failed_runs as u64))
            };
            format!("{lead} · {ran} · {outcome} · {}", describe_duration(total))
        }
    };
    (text, failed)
}

/// Whether the window closes by itself: every planned run ended with code 0.
pub(crate) fn succeeded(view: &OutputView) -> bool {
    view.finished
        .as_ref()
        .is_some_and(|finish| !finish.cancelled && finish.error.is_none())
        && !view.runs.is_empty()
        && view.runs.len() as u64 == view.planned
        && view
            .runs
            .iter()
            .all(|run| !run.stopped && run.exit.is_some_and(ProgramExit::success))
}

fn started(run: &OutputRun) -> String {
    chrono::DateTime::<chrono::Local>::from(run.started)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

struct Record {
    window: CommandOutputWindow,
    host: Rc<DialogHost>,
    rows: Rc<Rows>,
    /// The presenter revision shown, and the theme.
    revision: u64,
    theme: ThemeRevision,
    /// Close when finished successfully was acted on.
    closing: bool,
}

type Windows = Rc<RefCell<HashMap<UserCommandRequestId, Record>>>;

struct Context {
    main: slint::Weak<MainWindow>,
    state: std::rc::Weak<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
    windows: Windows,
    sessions: RefCell<DialogSessions>,
    closing: Rc<Cell<bool>>,
    failures: Failures<UserCommandRequestId>,
}

fn close_failed(
    p: &mut Presenter,
    id: UserCommandRequestId,
    send: &dyn Fn(Command) -> bool,
) -> bool {
    // Without a window the run must not continue unseen.
    p.close_command_output(id, send);
    true
}

pub(crate) fn install(
    main: &MainWindow,
    state: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
) {
    let closing = Rc::new(Cell::new(false));
    let failures = Failures::new(
        main,
        &state,
        &send,
        &closing,
        "Command output",
        close_failed,
    );
    let context = Rc::new(Context {
        main: main.as_weak(),
        state: Rc::downgrade(&state),
        send,
        windows: Rc::default(),
        sessions: RefCell::default(),
        closing,
        failures,
    });
    let hub = main.global::<CommandOutputHub>();
    let refreshing = context.clone();
    hub.on_refresh(move || refresh(&refreshing));
    hub.on_close_all(move || {
        context.closing.set(true);
        for (_, record) in std::mem::take(&mut *context.windows.borrow_mut()) {
            record.host.close(record.window.window());
        }
        // The engine's shutdown stops programs that still run.
    });
}

fn theme(p: &Presenter) -> ThemeRevision {
    let values = &p.preferences.values;
    (
        p.preferences.theme_index(),
        p.preferences.density_index(),
        values.font_family.clone(),
        i32::from(values.desktop_font_size),
        i32::from(values.touch_font_size),
    )
}

/// One presenter output window, as this module reads it.
struct Entry<'a> {
    session: &'a std::sync::Arc<UserCommandOutputSession>,
    revision: u64,
    status: &'a str,
    saving: bool,
    prompt: Option<String>,
}

fn refresh(context: &Rc<Context>) {
    if context.closing.get() {
        return;
    }
    let (Some(main), Some(state)) = (context.main.upgrade(), context.state.upgrade()) else {
        return;
    };
    let p = state.borrow();
    let shown: Vec<UserCommandRequestId> = p
        .command_launches
        .outputs
        .windows
        .iter()
        .map(|window| window.session.request)
        .collect();
    let removed: Vec<_> = context
        .windows
        .borrow()
        .keys()
        .filter(|id| !shown.contains(id))
        .copied()
        .collect();
    for id in removed {
        if let Some(record) = context.windows.borrow_mut().remove(&id) {
            record.host.close(record.window.window());
        }
    }
    for window in &p.command_launches.outputs.windows {
        let entry = Entry {
            session: &window.session,
            revision: window.revision,
            status: &window.status,
            saving: window.saving(),
            prompt: window.save_prompt(),
        };
        let entry = &entry;
        let id = entry.session.request;
        if context.failures.contains(id) {
            continue;
        }
        if !context.windows.borrow().contains_key(&id) {
            match create(context, &main, entry, &p) {
                Ok(record) => {
                    context.windows.borrow_mut().insert(id, record);
                }
                Err(error) => {
                    context.failures.record(id, error.to_string());
                    continue;
                }
            }
        }
        let mut windows = context.windows.borrow_mut();
        let Some(record) = windows.get_mut(&id) else {
            continue;
        };
        update(context, record, entry, &p);
    }
}

fn create(
    context: &Rc<Context>,
    main: &MainWindow,
    entry: &Entry<'_>,
    p: &Presenter,
) -> Result<Record, slint::PlatformError> {
    let session = context
        .sessions
        .borrow_mut()
        .allocate()
        .map_err(slint::PlatformError::from)?;
    let (window, host) =
        DialogHost::create_tool(session, DialogOwner::new(main), CommandOutputWindow::new)?;
    let host = Rc::new(host);
    crate::tooltip::install_output(&window)?;
    dialog_presentation::apply(&window, p);
    let rows = Rc::new(Rows::default());
    window.set_rows(ModelRc::from(rows.clone()));
    window.set_heading(crate::shown_text(&entry.session.name));
    // Every supported Windows version has Consolas; elsewhere the generic family resolves.
    if cfg!(windows) {
        window.set_mono_family("Consolas".into());
    }
    let id = entry.session.request;
    install_callbacks(context, &window, &host, session, id, &rows);
    host.show(window.window())?;
    window.invoke_begin();
    Ok(Record {
        window,
        host,
        rows,
        revision: 0,
        theme: theme(p),
        closing: false,
    })
}

fn install_callbacks(
    context: &Rc<Context>,
    window: &CommandOutputWindow,
    host: &Rc<DialogHost>,
    session: DialogSession,
    id: UserCommandRequestId,
    rows: &Rc<Rows>,
) {
    macro_rules! action {
        ($callback:ident, |$p:ident, $send:ident $(, $arg:ident)*| $body:block) => {{
            let weak = Rc::downgrade(context);
            let host = host.clone();
            window.$callback(move |$($arg),*| {
                let Some(context) = weak.upgrade() else { return; };
                if !host.is_current(session) { return; }
                let Some(state) = context.state.upgrade() else { return; };
                {
                    let mut presenter = state.borrow_mut();
                    let $p = &mut *presenter;
                    let $send = context.send.as_ref();
                    $body
                }
                if let Some(main) = context.main.upgrade() {
                    crate::refresh(&main, &state.borrow());
                }
            });
        }};
    }
    action!(on_stop, |p, send| {
        p.stop_command_output(id, send);
    });
    action!(on_copy_all, |p, send| {
        p.copy_command_output(id, send);
    });
    action!(on_clear, |p, _send| {
        p.clear_command_output(id);
    });
    action!(on_close_on_success_toggled, |p, send, close| {
        p.set_command_output_close_on_success(id, close, send);
    });
    action!(on_confirm_save, |p, send, replace| {
        p.confirm_command_output_save(id, replace, send);
    });
    {
        let window_weak = window.as_weak();
        action!(on_save_as, |p, send| {
            if let Some(window) = window_weak.upgrade() {
                let owner = crate::picker_window::native_path_picker_owner(window.window());
                p.save_command_output(id, owner, send);
            }
        });
    }
    {
        let weak = Rc::downgrade(context);
        let host = host.clone();
        let window_weak = window.as_weak();
        window.on_close_session(move || {
            let (Some(context), Some(window)) = (weak.upgrade(), window_weak.upgrade()) else {
                return;
            };
            close(&context, &host, &window, id);
        });
    }
    {
        let rows = rows.clone();
        let window_weak = window.as_weak();
        window.on_find(move |needle, backward| {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            match rows.find(&needle, window.get_match_row(), backward) {
                Some(index) => {
                    window.set_match_row(index as i32);
                    window.set_match_note(
                        format!("Line {} of {}", index + 1, rows.row_count()).into(),
                    );
                }
                None => {
                    window.set_match_row(-1);
                    window.set_match_note(if needle.is_empty() {
                        SharedString::default()
                    } else {
                        "Not found".into()
                    });
                }
            }
        });
    }
    {
        let window_weak = window.as_weak();
        window.on_find_closed(move || {
            if let Some(window) = window_weak.upgrade() {
                window.set_match_row(-1);
                window.set_match_note(SharedString::default());
            }
        });
    }
    let window_weak = window.as_weak();
    window.window().on_close_requested(move || {
        // The native close button: a running program stops with its window.
        if let Some(window) = window_weak.upgrade() {
            window.invoke_close_session();
        }
        slint::CloseRequestResponse::KeepWindowShown
    });
}

fn close(
    context: &Rc<Context>,
    host: &Rc<DialogHost>,
    window: &CommandOutputWindow,
    id: UserCommandRequestId,
) {
    let Some(state) = context.state.upgrade() else {
        return;
    };
    host.request_close(window.window(), || {
        state
            .borrow_mut()
            .close_command_output(id, context.send.as_ref());
        CloseDisposition::Hide
    });
    context.windows.borrow_mut().remove(&id);
    if let Some(main) = context.main.upgrade() {
        crate::refresh(&main, &state.borrow());
    }
}

fn update(context: &Rc<Context>, record: &mut Record, entry: &Entry<'_>, p: &Presenter) {
    let window = &record.window;
    if record.theme != theme(p) {
        dialog_presentation::apply(window, p);
        record.theme = theme(p);
    }
    let closes = p.command_output_closes_on_success(entry.session.request);
    window.set_close_on_success_available(closes.is_some());
    window.set_close_on_success(closes.unwrap_or(false));
    if record.revision == entry.revision {
        return;
    }
    record.revision = entry.revision;
    let view = entry.session.read_since(record.rows.next());
    record.rows.apply(&view);
    let (status, failed) = status(&view, &p.number_locale);
    window.set_status(status.into());
    window.set_failed(failed);
    window.set_running(view.finished.is_none());
    window.set_stopping(view.stopping);
    if let Some(run) = view.runs.last() {
        window.set_command_line(crate::shown_text(&run.command_line));
        window.set_directory(crate::shown_text(&run.directory));
    }
    if let Some(first) = view.runs.first() {
        window.set_started(started(first).into());
    }
    window.set_dropped(if view.dropped > 0 {
        format!(
            "Earlier output was dropped: {} lines. The window keeps the last {} lines.",
            p.number_locale.format_u64(view.dropped),
            p.number_locale.format_u64(MAX_OUTPUT_LINES as u64)
        )
        .into()
    } else {
        SharedString::default()
    });
    window.set_note(crate::shown_text(entry.status));
    window.set_saving(entry.saving);
    window.set_save_prompt(
        entry
            .prompt
            .as_deref()
            .map_or_else(SharedString::default, crate::shown_text),
    );
    window.set_columns(i32::try_from(record.rows.columns()).unwrap_or(i32::MAX));
    window.set_revision(window.get_revision().wrapping_add(1));
    if closes == Some(true) && succeeded(&view) && !record.closing {
        record.closing = true;
        // Outside the presenter borrow of this refresh.
        let weak = Rc::downgrade(context);
        let host = record.host.clone();
        let window = record.window.as_weak();
        let id = entry.session.request;
        slint::Timer::single_shot(Duration::ZERO, move || {
            if let (Some(context), Some(window)) = (weak.upgrade(), window.upgrade()) {
                close(&context, &host, &window, id);
            }
        });
    }
}
