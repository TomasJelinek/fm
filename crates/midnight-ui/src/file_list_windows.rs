//! An owned modal draft; all generation, clipboard and file work is delegated.
use crate::dialog_focus::{FocusReturn, restore_after_popup};
use crate::dialog_host::{DialogSession, DialogSessions};
use crate::presenter::file_list::{FileListSession, PRESETS, VARIABLES};
use crate::tool_window_failure::Failures;
use crate::{FileListPresentation, MainWindow, Presenter};
use midnight_core::*;
use slint::{ComponentHandle, Timer, TimerMode};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    time::Duration,
};

struct Record {
    host: Rc<EmbeddedLife>,
    shown: bool,
    revision: Option<u64>,
    path_revision: u64,
}
struct EmbeddedLife {
    main: slint::Weak<MainWindow>,
    session: DialogSession,
    token: slint::SharedString,
    open: Cell<bool>,
    focus: FocusReturn,
}
impl EmbeddedLife {
    fn is_current(&self, session: DialogSession, token: &str) -> bool {
        self.open.get() && self.session == session && self.token.as_str() == token
    }
    fn close(&self) {
        if !self.open.replace(false) {
            return;
        }
        if let Some(main) = self.main.upgrade() {
            let window = main.global::<FileListPresentation>();
            if window.get_token() == self.token {
                window.set_active(false);
                let weak = main.as_weak();
                let token = self.token.clone();
                restore_after_popup(&main, Some(self.focus.clone()), move || {
                    weak.upgrade().is_some_and(|main| {
                        let window = main.global::<FileListPresentation>();
                        window.get_token() == token && !window.get_active()
                    })
                });
            }
        }
    }
}
type Windows = Rc<RefCell<HashMap<DesktopRequestId, Record>>>;

pub(crate) fn install(
    main: &MainWindow,
    state: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
) {
    let windows: Windows = Rc::default();
    let sessions = RefCell::new(DialogSessions::default());
    let closing = Rc::new(Cell::new(false));
    let failures = Failures::new(
        main,
        &state,
        &send,
        &closing,
        "Make File List",
        Presenter::close_file_list,
    );
    {
        let state = state.clone();
        let send = send.clone();
        let weak = main.as_weak();
        let closing = closing.clone();
        main.on_make_file_list(move |pane| {
            if closing.get() {
                return;
            }
            state
                .borrow_mut()
                .request_make_file_list(crate::pane(pane), send.as_ref());
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
        main.on_refresh_file_list_window(move || {
            if closing.get() {
                return;
            }
            if let Some(main) = weak.upgrade() {
                refresh(&main, &windows, &sessions, &state, &send, &failures);
            }
        });
    }
    main.on_close_file_list_window(move || {
        closing.set(true);
        for (_, record) in std::mem::take(&mut *windows.borrow_mut()) {
            record.host.close();
        }
        // The engine shutdown barrier owns cancellation. Do not borrow Presenter here:
        // application shutdown can already hold its mutable borrow.
    });
}

fn refresh(
    main: &MainWindow,
    windows: &Windows,
    sessions: &RefCell<DialogSessions>,
    state: &Rc<RefCell<Presenter>>,
    send: &Rc<dyn Fn(Command) -> bool>,
    failures: &Failures<DesktopRequestId>,
) {
    let id = state.borrow().file_list.session.as_ref().map(|s| s.id);
    let removed: Vec<_> = windows
        .borrow()
        .keys()
        .copied()
        .filter(|key| Some(*key) != id)
        .collect();
    for key in removed {
        if let Some(record) = windows.borrow_mut().remove(&key) {
            record.host.close();
        }
    }
    let Some(id) = id else { return };
    if failures.contains(id) {
        return;
    }
    if !windows.borrow().contains_key(&id) {
        match create(main, id, sessions, state, send) {
            Ok(record) => {
                windows.borrow_mut().insert(id, record);
            }
            Err(error) => {
                failures.record(id, error.to_string());
                return;
            }
        }
    }
    let p = state.borrow();
    let Some(s) = &p.file_list.session else {
        return;
    };
    let mut records = windows.borrow_mut();
    let Some(record) = records.get_mut(&id) else {
        return;
    };
    let window = main.global::<FileListPresentation>();
    if record.revision == Some(s.revision) {
        return;
    }
    record.revision = Some(s.revision);
    sync(&window, s, &p.number_locale);
    if record.path_revision != s.path_revision {
        window.set_destination_path(s.path.to_string_lossy().as_ref().into());
        record.path_revision = s.path_revision;
    }
    if !record.shown {
        record.shown = true;
        window.invoke_begin();
    }
}

fn sync(window: &FileListPresentation<'_>, s: &FileListSession, locale: &NumberLocale) {
    window.set_busy(s.busy());
    window.set_preview_text(s.preview.clone().into());
    window.set_summary(s.summary.clone().into());
    window.set_error_text(s.error.clone().into());
    window.set_conflict_pending(s.can_confirm());
    if let Some(entry) = &s.conflict {
        let item = &entry.item;
        let bytes = item.metadata.size.map_or_else(
            || "Unavailable".into(),
            |bytes| format!("{} bytes", locale.format_u64(bytes)),
        );
        let modified = crate::time_presentation::modified(&item.metadata).map_or_else(
            || "Unavailable".into(),
            |v| format!("{} {}", v.date, v.time),
        );
        window.set_conflict_text(
            format!(
                "{}\n\nSize: {bytes}\nModified: {modified}",
                item.path.display()
            )
            .into(),
        );
        window.set_append_output(s.append);
    }
}

fn create(
    main: &MainWindow,
    id: DesktopRequestId,
    sessions: &RefCell<DialogSessions>,
    state: &Rc<RefCell<Presenter>>,
    send: &Rc<dyn Fn(Command) -> bool>,
) -> Result<Record, slint::PlatformError> {
    let session = sessions
        .borrow_mut()
        .allocate()
        .map_err(slint::PlatformError::from)?;
    main.invoke_drag_invalidate_targets();
    let token: slint::SharedString = format!("{session:?}").into();
    let host = Rc::new(EmbeddedLife {
        session,
        token: token.clone(),
        main: main.as_weak(),
        open: Cell::new(true),
        focus: FocusReturn::capture(main.window()),
    });
    let window = main.global::<FileListPresentation>();
    window.set_token(token);
    window.set_variable_labels(
        Rc::new(slint::VecModel::from(
            VARIABLES
                .iter()
                .map(|(label, _)| slint::SharedString::from(*label))
                .collect::<Vec<_>>(),
        ))
        .into(),
    );
    if let Some(s) = &state.borrow().file_list.session {
        window.set_template_text(s.template.clone().into());
        window.set_destination(s.destination);
        window.set_destination_path(s.path.to_string_lossy().as_ref().into());
        window.set_append_output(s.append);
        window.set_template_history(
            Rc::new(slint::VecModel::from(
                s.history
                    .iter()
                    .map(|text| slint::SharedString::from(text.as_str()))
                    .collect::<Vec<_>>(),
            ))
            .into(),
        );
        sync(&window, s, &state.borrow().number_locale);
    }
    let timer = Rc::new(Timer::default());
    {
        let state = state.clone();
        let send = send.clone();
        let main = main.as_weak();
        let timer = timer.clone();
        let host = host.clone();
        window.on_template_edited(move |token, text| {
            if !host.is_current(session, token.as_str()) {
                return;
            }
            edit(&state, &send, &main, &timer, id, text.to_string());
        });
    }
    {
        let state = state.clone();
        let send = send.clone();
        let main = main.as_weak();
        let weak = main.clone();
        let timer = timer.clone();
        let host = host.clone();
        window.on_insert_variable(move |token, index| {
            if !host.is_current(session, token.as_str()) {
                return;
            }
            let Some((_, variable)) = usize::try_from(index).ok().and_then(|i| VARIABLES.get(i))
            else {
                return;
            };
            let Some(owner) = weak.upgrade() else { return };
            let window = owner.global::<FileListPresentation>();
            let Some((text, caret)) = insert_variable(
                &window.get_template_text(),
                window.get_template_anchor(),
                window.get_template_cursor(),
                variable,
            ) else {
                return;
            };
            window.set_template_text(text.clone().into());
            edit(&state, &send, &main, &timer, id, text);
            window.invoke_focus_template_at(caret);
        });
    }
    {
        let state = state.clone();
        let send = send.clone();
        let main = main.as_weak();
        let weak = main.clone();
        let timer = timer.clone();
        let host = host.clone();
        window.on_select_preset(move |token, index| {
            if !host.is_current(session, token.as_str()) {
                return;
            }
            let Some(text) = usize::try_from(index).ok().and_then(|i| PRESETS.get(i)) else {
                return;
            };
            let Some(owner) = weak.upgrade() else { return };
            let window = owner.global::<FileListPresentation>();
            window.set_template_text((*text).into());
            edit(&state, &send, &main, &timer, id, (*text).into());
            window.invoke_focus_template_at(text.len() as i32);
        });
    }
    {
        let state = state.clone();
        let send = send.clone();
        let main = main.as_weak();
        let weak = main.clone();
        let timer = timer.clone();
        let host = host.clone();
        window.on_select_history(move |token, index| {
            if !host.is_current(session, token.as_str()) {
                return;
            }
            let text = state
                .borrow()
                .file_list
                .session
                .as_ref()
                .filter(|s| s.id == id)
                .and_then(|s| usize::try_from(index).ok().and_then(|i| s.history.get(i)))
                .cloned();
            let (Some(text), Some(owner)) = (text, weak.upgrade()) else {
                return;
            };
            let window = owner.global::<FileListPresentation>();
            window.set_template_text(text.clone().into());
            let caret = text.len() as i32;
            edit(&state, &send, &main, &timer, id, text);
            window.invoke_focus_template_at(caret);
        });
    }
    macro_rules! callback {
        ($name:ident, |$p:ident, $send:ident, $window:ident $(, $arg:ident)*| $body:block) => {{
            let state = state.clone(); let sender = send.clone();
            let main = main.as_weak(); let weak = main.clone();
            let host = host.clone();
            window.$name(move |token $(, $arg)*| {
                if !host.is_current(session, token.as_str()) { return; }
                let Some(owner) = weak.upgrade() else { return };
                let $window = owner.global::<FileListPresentation>();
                { let mut p = state.borrow_mut(); let $p = &mut *p; let $send = sender.as_ref(); $body }
                if let Some(main) = main.upgrade() { crate::refresh(&main, &state.borrow()); }
            });
        }};
    }
    callback!(on_generate, |p, send, window| {
        p.generate_file_list(
            id,
            window.get_destination(),
            window.get_destination_path().to_string(),
            window.get_append_output(),
            send,
        );
    });
    {
        let host = host.clone();
        let state = state.clone();
        let send = send.clone();
        let weak = main.as_weak();
        window.on_browse(move |token| {
            if !host.is_current(session, token.as_str()) {
                return;
            }
            let Some(main) = weak.upgrade() else { return };
            state.borrow_mut().request_file_list_picker(
                id,
                crate::picker_window::native_path_picker_owner(main.window()),
                send.as_ref(),
            );
            crate::refresh(&main, &state.borrow());
        });
    }
    {
        let host = host.clone();
        let state = state.clone();
        let send = send.clone();
        let main = main.as_weak();
        window.on_cancel(move |token| {
            if !host.is_current(session, token.as_str()) {
                return;
            }
            let closed = {
                let mut p = state.borrow_mut();
                p.close_file_list(id, send.as_ref());
                !p.file_list.session.as_ref().is_some_and(|s| s.id == id)
            };
            if closed {
                host.close();
            }
            if let Some(main) = main.upgrade() {
                crate::refresh(&main, &state.borrow());
            }
        });
    }
    callback!(on_confirm, |p, send, _window| {
        p.confirm_file_list(id, send);
    });
    callback!(on_copy_error, |p, send, _window, text| {
        p.copy_viewer_text(text.to_string(), send);
    });
    Ok(Record {
        host,
        shown: false,
        revision: None,
        path_revision: 0,
    })
}

fn edit(
    state: &Rc<RefCell<Presenter>>,
    send: &Rc<dyn Fn(Command) -> bool>,
    main: &slint::Weak<MainWindow>,
    timer: &Timer,
    id: DesktopRequestId,
    text: String,
) {
    state.borrow_mut().edit_file_list_template(id, text);
    if let Some(main) = main.upgrade() {
        crate::refresh(&main, &state.borrow());
    }
    let state = Rc::downgrade(state);
    let main = main.clone();
    let send = send.clone();
    timer.start(
        TimerMode::SingleShot,
        Duration::from_millis(180),
        move || {
            let Some(state) = state.upgrade() else { return };
            state.borrow_mut().preview_file_list(id, send.as_ref());
            if let Some(main) = main.upgrade() {
                crate::refresh(&main, &state.borrow());
            }
        },
    );
}

fn insert_variable(text: &str, anchor: i32, cursor: i32, variable: &str) -> Option<(String, i32)> {
    let start = usize::try_from(anchor.min(cursor)).ok()?;
    let end = usize::try_from(anchor.max(cursor)).ok()?;
    if end > text.len() || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
        return None;
    }
    let mut edited = String::with_capacity(text.len() + variable.len());
    edited.push_str(&text[..start]);
    edited.push_str(variable);
    let caret = i32::try_from(edited.len()).ok()?;
    edited.push_str(&text[end..]);
    Some((edited, caret))
}

#[cfg(test)]
mod tests {
    use super::insert_variable;

    #[test]
    fn insertion_uses_utf8_selection_and_rejects_mid_character_offsets() {
        assert_eq!(
            insert_variable("文档name.txt", 6, 10, "$(FileName)"),
            Some(("文档$(FileName).txt".into(), 17))
        );
        assert_eq!(insert_variable("文档", 1, 3, "x"), None);
        assert_eq!(
            insert_variable("文档", 6, 3, "$(TAB)"),
            Some(("文$(TAB)".into(), 9))
        );
    }
}
