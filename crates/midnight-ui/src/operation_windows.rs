//! Embedded confirmation lifetime. Records contain UI identity, never captured providers.
use crate::{
    BeginOwned, MainWindow, OperationPresentation, Presenter, copy_draft_windows,
    dialog_focus::{FocusReturn, restore_after_popup},
    dialog_host::{DialogSession, DialogSessions},
    tool_window_failure::Failures,
};
use midnight_core::{Command, MutationKind, OperationId};
use slint::{ComponentHandle, PlatformError, SharedString};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

#[derive(Clone, PartialEq, Eq)]
enum Key {
    Operation(OperationId),
    Copy(String),
}
impl Key {
    fn current(&self, p: &Presenter) -> bool {
        if p.operations.closing {
            return false;
        }
        match self {
            Self::Operation(id) => {
                p.operations.pending == Some(*id)
                    && p.operations.preview.as_ref().is_some_and(|preview| {
                        preview.id == *id && preview.kind != MutationKind::EditMetadata
                    })
            }
            Self::Copy(token) => p
                .copies
                .draft
                .as_ref()
                .is_some_and(|draft| draft.token == *token && !draft.retire_requested),
        }
    }
}
struct Record {
    key: Key,
    session: DialogSession,
    token: SharedString,
    main: slint::Weak<MainWindow>,
    focus: FocusReturn,
    error: RefCell<String>,
    cancelling: Cell<bool>,
    retry: slint::Timer,
}
#[derive(Default)]
struct Windows {
    sessions: DialogSessions,
    record: Option<Rc<Record>>,
    failed_copy: Option<String>,
}
struct Context {
    main: slint::Weak<MainWindow>,
    state: std::rc::Weak<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
    begin_owned: BeginOwned,
    windows: RefCell<Windows>,
    closing: Rc<Cell<bool>>,
    failures: Failures<OperationId>,
}
fn cancel_prepared(p: &mut Presenter, id: OperationId, send: &dyn Fn(Command) -> bool) -> bool {
    if !Key::Operation(id).current(p) {
        return true;
    }
    p.cancel_operation(&id.0.to_string(), send);
    !Key::Operation(id).current(p)
}

pub(crate) fn install(
    main: &MainWindow,
    state: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
    begin_owned: BeginOwned,
) {
    let closing = Rc::new(Cell::new(false));
    let context = Rc::new(Context {
        main: main.as_weak(),
        state: Rc::downgrade(&state),
        send: send.clone(),
        begin_owned,
        windows: RefCell::new(Windows::default()),
        failures: Failures::new(
            main,
            &state,
            &send,
            &closing,
            "Operation window",
            cancel_prepared,
        ),
        closing,
    });
    let ctx = context.clone();
    main.on_refresh_operation_windows(move || {
        let (Some(main), Some(state)) = (ctx.main.upgrade(), ctx.state.upgrade()) else {
            return;
        };
        synchronize(&ctx, &main, &state.borrow());
    });
    main.on_close_operation_windows(move || {
        context.closing.set(true);
        close_record(context.windows.borrow_mut().record.take());
        // Copy drafts are local, provider-bearing captures: their existing cancel
        // route transfers ownership to backend reclamation instead of dropping here.
        if let Some(state) = context.state.upgrade()
            && crate::with_owned_commands(&state, &context.begin_owned, |p, _| {
                if let Some(token) = p.copies.draft.as_ref().map(|draft| draft.token.clone()) {
                    p.cancel_copy_dialog(&token);
                }
            })
            .is_none()
            && let Ok(mut p) = state.try_borrow_mut()
            && let Some(token) = p.copies.draft.as_ref().map(|draft| draft.token.clone())
        {
            p.defer_cancel_copy_dialog(&token);
        }
    });
}
fn synchronize(ctx: &Rc<Context>, main: &MainWindow, p: &Presenter) {
    let old = {
        let mut windows = ctx.windows.borrow_mut();
        if windows
            .record
            .as_ref()
            .is_some_and(|record| !record.key.current(p) || ctx.closing.get())
        {
            windows.record.take()
        } else {
            None
        }
    };
    close_record(old);
    if ctx.closing.get() || p.operations.closing {
        return;
    }
    if let Some(record) = ctx.windows.borrow().record.clone() {
        update_warning(&record, main);
        return;
    }
    if !main.window().is_visible() {
        return;
    }
    let key = p
        .operations
        .preview
        .as_ref()
        .filter(|preview| preview.kind != MutationKind::EditMetadata)
        .map(|preview| Key::Operation(preview.id))
        .or_else(|| {
            p.copies
                .draft
                .as_ref()
                .map(|draft| Key::Copy(draft.token.clone()))
        });
    let Some(key) = key.filter(|key| key.current(p)) else {
        return;
    };
    match &key {
        Key::Operation(id) if ctx.failures.contains(*id) => return,
        Key::Copy(token) if ctx.windows.borrow().failed_copy.as_ref() == Some(token) => return,
        _ => {}
    }
    match open(ctx, main, p, key.clone()) {
        Ok(record) => ctx.windows.borrow_mut().record = Some(Rc::new(record)),
        Err(error) => failed(ctx, key, error.to_string()),
    }
}
fn open(
    ctx: &Rc<Context>,
    main: &MainWindow,
    p: &Presenter,
    key: Key,
) -> Result<Record, PlatformError> {
    main.invoke_drag_invalidate_targets();
    let session = ctx.windows.borrow_mut().sessions.allocate()?;
    let token: SharedString = format!("{session:?}").into();
    let focus = FocusReturn::capture(main.window());
    let window = main.global::<OperationPresentation>();
    window.set_token(token.clone());
    // Unlike a new native component, this shared form must clear optional state.
    window.set_kind(-1);
    window.set_confirm_enabled(true);
    window.set_option_visible(false);
    window.set_policy("".into());
    window.set_wait_visible(false);
    window.set_wait_checked(false);
    window.set_follow_directory_links_visible(false);
    window.set_option_enabled(true);
    window.set_option_checked(false);
    window.set_dangerous(false);
    match &key {
        Key::Copy(_) => copy_draft_windows::initialize(
            &window,
            main,
            p.copies
                .draft
                .as_ref()
                .is_some_and(|draft| draft.directory_link_options_available()),
        ),
        Key::Operation(_) => {
            window.set_title_text(main.get_operation_title());
            window.set_kind(main.get_operation_kind());
            window.set_confirm_text(main.get_operation_title());
            window.set_body(main.get_operation_body());
            window.set_initial_name(main.get_operation_name());
            window.set_selection_end(main.get_operation_selection_end());
            window.set_has_name(main.get_operation_has_name());
            window.set_input_label(
                match main.get_operation_kind() {
                    0 => "New name",
                    3 => "Folder name or path",
                    4 => "File name",
                    _ => "",
                }
                .into(),
            );
            window.set_dangerous(main.get_operation_dangerous());
            window.set_native_icon(main.get_operation_icon());
        }
    }
    let weak = Rc::downgrade(ctx);
    let expected = token.clone();
    window.on_confirm(move |token, value| {
        if token != expected {
            return;
        }
        if let Some(ctx) = weak.upgrade() {
            confirm(&ctx, session, value);
        }
    });
    let weak = Rc::downgrade(ctx);
    let expected = token.clone();
    window.on_request_dismiss(move |token| {
        if token != expected {
            return;
        }
        if let Some(ctx) = weak.upgrade() {
            dismiss(&ctx, session);
        }
    });
    let record = Record {
        key,
        session,
        token,
        main: main.as_weak(),
        focus,
        error: RefCell::new(String::new()),
        cancelling: Cell::new(false),
        retry: slint::Timer::default(),
    };
    update_warning(&record, main);
    window.invoke_begin();
    Ok(record)
}
fn update_warning(record: &Record, main: &MainWindow) {
    let base = match record.key {
        Key::Operation(_) => main.get_operation_warning(),
        Key::Copy(_) => main.get_copy_draft_error(),
    };
    let error = record.error.borrow();
    let warning = if error.is_empty() || base.as_str() == error.as_str() {
        base
    } else if base.is_empty() {
        error.as_str().into()
    } else {
        format!("{base}\n\n{error}").into()
    };
    main.global::<OperationPresentation>().set_warning(warning);
}
fn current(ctx: &Context, session: DialogSession) -> Option<Rc<Record>> {
    ctx.windows
        .borrow()
        .record
        .as_ref()
        .filter(|record| record.session == session)
        .cloned()
}
fn confirm(ctx: &Rc<Context>, session: DialogSession, value: SharedString) {
    let (Some(record), Some(state), Some(main)) = (
        current(ctx, session),
        ctx.state.upgrade(),
        ctx.main.upgrade(),
    ) else {
        return;
    };
    if record.cancelling.get() {
        return;
    }
    let accepted = match &record.key {
        Key::Operation(id) => {
            let mut p = state.borrow_mut();
            if !record.key.current(&p) {
                return;
            }
            *record.error.borrow_mut() = {
                p.confirm_operation(&id.0.to_string(), value.to_string(), ctx.send.as_ref());
                "The operation queue is busy. Try again.".into()
            };
            !record.key.current(&p)
        }
        Key::Copy(token) => {
            match crate::with_owned_commands(&state, &ctx.begin_owned, |p, send| {
                if !record.key.current(p) {
                    return false;
                }
                let window = main.global::<OperationPresentation>();
                let follow_directory_links = window
                    .get_follow_directory_links_visible()
                    .then(|| window.get_follow_directory_links_checked());
                *record.error.borrow_mut() = copy_draft_windows::confirm(
                    p,
                    token,
                    value.as_str(),
                    window.get_option_checked(),
                    window.get_wait_checked(),
                    follow_directory_links,
                    send,
                );
                !record.key.current(p)
            }) {
                Some(accepted) => accepted,
                None => {
                    if let Ok(mut p) = state.try_borrow_mut() {
                        *record.error.borrow_mut() = p.copy_dialog_reclaim_busy(token);
                    }
                    false
                }
            }
        }
    };
    if accepted {
        remove(ctx, session);
    }
    if let Ok(p) = state.try_borrow() {
        crate::refresh(&main, &p);
    }
}
fn dismiss(ctx: &Rc<Context>, session: DialogSession) {
    let (Some(record), Some(state), Some(main)) = (
        current(ctx, session),
        ctx.state.upgrade(),
        ctx.main.upgrade(),
    ) else {
        return;
    };
    let accepted = match &record.key {
        Key::Copy(token) => {
            match crate::with_owned_commands(&state, &ctx.begin_owned, |p, _| {
                p.cancel_copy_dialog(token);
                true
            }) {
                Some(accepted) => accepted,
                None => {
                    let Ok(mut p) = state.try_borrow_mut() else {
                        wait_to_cancel(ctx, &record);
                        update_warning(&record, &main);
                        return;
                    };
                    p.defer_cancel_copy_dialog(token);
                    true
                }
            }
        }
        Key::Operation(id) => {
            let Ok(mut p) = state.try_borrow_mut() else {
                wait_to_cancel(ctx, &record);
                update_warning(&record, &main);
                return;
            };
            if !record.key.current(&p) {
                return;
            }
            cancel_prepared(&mut p, *id, ctx.send.as_ref())
        }
    };
    if accepted {
        remove(ctx, session);
    } else {
        wait_to_cancel(ctx, &record);
    }
    if let Ok(p) = state.try_borrow() {
        crate::refresh(&main, &p);
    }
}
fn wait_to_cancel(ctx: &Rc<Context>, record: &Record) {
    *record.error.borrow_mut() = "Waiting to cancel…".into();
    if let Some(main) = record.main.upgrade() {
        main.global::<OperationPresentation>()
            .set_confirm_enabled(false);
    }
    if !record.cancelling.replace(true) {
        let weak = Rc::downgrade(ctx);
        let session = record.session;
        record.retry.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(250),
            move || {
                if let Some(ctx) = weak.upgrade() {
                    dismiss(&ctx, session);
                }
            },
        );
    }
}
fn remove(ctx: &Context, session: DialogSession) {
    let record = {
        let mut windows = ctx.windows.borrow_mut();
        if windows
            .record
            .as_ref()
            .is_some_and(|record| record.session == session)
        {
            windows.record.take()
        } else {
            None
        }
    };
    close_record(record);
}
fn close_record(record: Option<Rc<Record>>) {
    if let Some(record) = record {
        record.retry.stop();
        if let Some(main) = record.main.upgrade() {
            let window = main.global::<OperationPresentation>();
            if window.get_token() == record.token {
                window.set_active(false);
                let weak = main.as_weak();
                let token = record.token.clone();
                restore_after_popup(&main, Some(record.focus.clone()), move || {
                    weak.upgrade().is_some_and(|main| {
                        let window = main.global::<OperationPresentation>();
                        window.get_token() == token && !window.get_active()
                    })
                });
            }
        }
    }
}
fn failed(ctx: &Rc<Context>, key: Key, message: String) {
    match key {
        Key::Operation(id) => ctx.failures.record(id, message),
        Key::Copy(token) => {
            ctx.windows.borrow_mut().failed_copy = Some(token.clone());
            retry_failed_copy(ctx, token, message, Duration::ZERO);
        }
    }
}
fn retry_failed_copy(ctx: &Rc<Context>, token: String, message: String, delay: Duration) {
    let weak = Rc::downgrade(ctx);
    slint::Timer::single_shot(delay, move || {
        let Some(ctx) = weak.upgrade() else {
            return;
        };
        let (Some(state), Some(main)) = (ctx.state.upgrade(), ctx.main.upgrade()) else {
            return;
        };
        if ctx.closing.get() || ctx.windows.borrow().failed_copy.as_ref() != Some(&token) {
            return;
        }
        if crate::with_owned_commands(&state, &ctx.begin_owned, |p, _| {
            if Key::Copy(token.clone()).current(p) {
                p.cancel_copy_dialog(&token);
                p.desktop.report("Copy/Move window", message.clone());
            }
        })
        .is_none()
        {
            let Ok(mut p) = state.try_borrow_mut() else {
                retry_failed_copy(&ctx, token, message, Duration::from_millis(250));
                return;
            };
            if Key::Copy(token.clone()).current(&p) {
                p.defer_cancel_copy_dialog(&token);
                p.desktop.report("Copy/Move window", message);
            }
        }
        if let Ok(p) = state.try_borrow() {
            crate::refresh(&main, &p);
        }
    });
}

#[cfg(test)]
#[path = "operation_window_lifetime_tests.rs"]
mod lifetime_tests;
