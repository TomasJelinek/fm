//! A native modeless window; all generation, clipboard and file work is delegated.
use crate::dialog_host::{CloseDisposition, DialogHost, DialogOwner, DialogSessions};
use crate::presenter::file_list::{FileListSession, PRESETS, VARIABLES};
use crate::tool_window_failure::Failures;
use crate::{FileListWindow, MainWindow, Presenter, Theme};
use midnight_core::*;
use slint::{ComponentHandle, Timer, TimerMode};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    time::Duration,
};

struct Record {
    window: FileListWindow,
    host: Rc<DialogHost>,
    shown: bool,
    revision: Option<u64>,
    path_revision: u64,
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
        let windows = windows.clone();
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
            for record in windows.borrow().values() {
                let _ = record.host.show(record.window.window());
                record.window.invoke_begin();
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
            record.host.close(record.window.window());
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
            record.host.close(record.window.window());
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
    apply_theme(main, &record.window);
    crate::tooltip::synchronize_file_list(&record.window);
    if record.revision == Some(s.revision) {
        return;
    }
    record.revision = Some(s.revision);
    sync(&record.window, s, &p.number_locale);
    if record.path_revision != s.path_revision {
        record
            .window
            .set_destination_path(s.path.to_string_lossy().as_ref().into());
        record.path_revision = s.path_revision;
    }
    let shown = if record.shown {
        Ok(())
    } else {
        record.host.show(record.window.window()).map(|()| {
            record.shown = true;
            record.window.invoke_begin();
        })
    };
    drop(records);
    drop(p);
    if let Err(error) = shown {
        windows.borrow_mut().remove(&id);
        failures.record(id, error.to_string());
    }
}

fn apply_theme(main: &MainWindow, window: &FileListWindow) {
    let theme = main.global::<Theme>();
    window.invoke_apply_preferences(
        theme.get_scheme(),
        theme.get_density(),
        theme.get_font_family(),
        theme.get_desktop_font_size(),
        theme.get_touch_font_size(),
    );
}

fn sync(window: &FileListWindow, s: &FileListSession, locale: &NumberLocale) {
    window.set_busy(s.busy());
    window.set_preview_text(s.preview.clone().into());
    window.set_summary(s.summary.clone().into());
    window.set_error_text(s.error.clone().into());
    window.set_conflict_pending(s.conflict.is_some());
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
    let (window, host) =
        DialogHost::create_tool(session, DialogOwner::new(main), FileListWindow::new)?;
    let host = Rc::new(host);
    crate::tooltip::install_file_list(&window)?;
    apply_theme(main, &window);
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
        window.on_template_edited(move |text| {
            if !host.is_current(session) {
                return;
            }
            edit(&state, &send, &main, &timer, id, text.to_string());
        });
    }
    {
        let state = state.clone();
        let send = send.clone();
        let main = main.as_weak();
        let weak = window.as_weak();
        let timer = timer.clone();
        let host = host.clone();
        window.on_insert_variable(move |index| {
            if !host.is_current(session) {
                return;
            }
            let Some((_, variable)) = usize::try_from(index).ok().and_then(|i| VARIABLES.get(i))
            else {
                return;
            };
            let Some(window) = weak.upgrade() else { return };
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
        let weak = window.as_weak();
        let timer = timer.clone();
        let host = host.clone();
        window.on_select_preset(move |index| {
            if !host.is_current(session) {
                return;
            }
            let Some(text) = usize::try_from(index).ok().and_then(|i| PRESETS.get(i)) else {
                return;
            };
            let Some(window) = weak.upgrade() else { return };
            window.set_template_text((*text).into());
            edit(&state, &send, &main, &timer, id, (*text).into());
            window.invoke_focus_template_at(text.len() as i32);
        });
    }
    {
        let state = state.clone();
        let send = send.clone();
        let main = main.as_weak();
        let weak = window.as_weak();
        let timer = timer.clone();
        let host = host.clone();
        window.on_select_history(move |index| {
            if !host.is_current(session) {
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
            let (Some(text), Some(window)) = (text, weak.upgrade()) else {
                return;
            };
            window.set_template_text(text.clone().into());
            let caret = text.len() as i32;
            edit(&state, &send, &main, &timer, id, text);
            window.invoke_focus_template_at(caret);
        });
    }
    macro_rules! callback {
        ($name:ident, |$p:ident, $send:ident, $window:ident $(, $arg:ident)*| $body:block) => {{
            let state = state.clone(); let sender = send.clone();
            let main = main.as_weak(); let weak = window.as_weak();
            let host = host.clone();
            window.$name(move |$($arg),*| {
                if !host.is_current(session) { return; }
                let Some($window) = weak.upgrade() else { return };
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
    callback!(on_browse, |p, send, window| {
        p.request_file_list_picker(
            id,
            crate::picker_window::native_path_picker_owner(window.window()),
            send,
        );
    });
    {
        let host = host.clone();
        let state = state.clone();
        let send = send.clone();
        let weak = window.as_weak();
        let main = main.as_weak();
        window.on_cancel(move || {
            let Some(window) = weak.upgrade() else { return };
            host.request_close(window.window(), || {
                let mut p = state.borrow_mut();
                p.close_file_list(id, send.as_ref());
                if p.file_list.session.as_ref().is_some_and(|s| s.id == id) {
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
    callback!(on_confirm, |p, send, _window| {
        p.confirm_file_list(id, send);
    });
    callback!(on_copy_error, |p, send, _window, text| {
        p.copy_viewer_text(text.to_string(), send);
    });
    {
        let weak = window.as_weak();
        window.window().on_close_requested(move || {
            if let Some(window) = weak.upgrade() {
                window.invoke_cancel();
            }
            slint::CloseRequestResponse::KeepWindowShown
        });
    }
    Ok(Record {
        window,
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
