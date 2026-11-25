//! Embedded confirmation lifetime. Records contain UI identity, never captured providers.
use crate::{
    BeginOwned, MainWindow, OperationPresentation, Presenter, copy_draft_windows,
    dialog_focus::{FocusReturn, restore_after_popup},
    dialog_host::{DialogSession, DialogSessions},
    tool_window_failure::Failures,
};
use midnight_core::{Command, DesktopRequestId, MutationKind, OperationId};
use slint::{ComponentHandle, PlatformError, SharedString};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

#[derive(Clone, PartialEq, Eq)]
enum Key {
    Operation(OperationId, String),
    Copy(String),
}
impl Key {
    fn current(&self, p: &Presenter) -> bool {
        if p.operations.closing {
            return false;
        }
        match self {
            Self::Operation(id, token) => p.operation_form().is_some_and(|(preview, current)| {
                preview.id == *id && preview.kind != MutationKind::EditMetadata && current == *token
            }),
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
    displayed_pick: Cell<Option<DesktopRequestId>>,
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
    let Some((_, token)) = p.operation_form().filter(|(preview, _)| preview.id == id) else {
        return true;
    };
    p.cancel_operation(&token, send);
    !Key::Operation(id, token).current(p)
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
                    p.cancel_copy_destination_picker(context.send.as_ref());
                    p.cancel_copy_dialog(&token);
                }
            })
            .is_none()
            && let Ok(mut p) = state.try_borrow_mut()
            && let Some(token) = p.copies.draft.as_ref().map(|draft| draft.token.clone())
        {
            p.cancel_copy_destination_picker(context.send.as_ref());
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
        if matches!(record.key, Key::Operation(..))
            && p.operations
                .preview
                .as_ref()
                .is_some_and(|preview| preview.kind == MutationKind::CreateFile)
        {
            // New empty file can become Replace file while its form is open.
            let window = main.global::<OperationPresentation>();
            window.set_title_text(main.get_operation_title());
            window.set_confirm_text(main.get_operation_title());
            window.set_dangerous(main.get_operation_dangerous());
        }
        if matches!(record.key, Key::Copy(_)) {
            let form = main.global::<OperationPresentation>();
            let picking = p.copies.destination_picker.is_some();
            form.set_picking_destination(picking);
            form.set_confirm_enabled(!picking && !record.cancelling.get());
            if let Some((id, path)) = &p.copies.picked_destination
                && record.displayed_pick.replace(Some(*id)) != Some(*id)
                && !record.cancelling.get()
            {
                form.set_field_value(path.to_string_lossy().as_ref().into());
            }
            // The destination's link facts can arrive while the dialog is open.
            copy_draft_windows::refresh_links(&form, p.copy_draft_links_view().as_ref());
        }
        update_warning(&record, main);
        return;
    }
    if !main.window().is_visible() {
        return;
    }
    let key = p
        .operation_form()
        .filter(|(preview, _)| preview.kind != MutationKind::EditMetadata)
        .map(|(preview, token)| Key::Operation(preview.id, token))
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
        Key::Operation(id, _) if ctx.failures.contains(*id) => return,
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
    window.set_picking_destination(false);
    window.set_option_visible(false);
    window.set_body_hint("".into());
    window.set_policy("".into());
    window.set_wait_visible(false);
    window.set_wait_checked(false);
    window.set_links_visible(false);
    window.set_option_enabled(true);
    window.set_option_checked(false);
    window.set_dangerous(false);
    let (delete, folder) = match &key {
        Key::Operation(..) => (
            crate::operation_presentation::delete_form(p),
            crate::operation_presentation::folder_location(p),
        ),
        Key::Copy(_) => (None, None),
    };
    window.set_question(
        delete
            .as_ref()
            .map(|form| form.question.clone())
            .unwrap_or_default(),
    );
    window.set_detail_labels(
        delete
            .as_ref()
            .map(|form| form.labels.clone())
            .unwrap_or_default(),
    );
    // A path's tooltip may still show from an earlier confirmation's rows.
    crate::tooltip::dismiss(main.window());
    window.set_folder_location(
        folder
            .map(|path| crate::shown_text(&path.to_string_lossy()))
            .unwrap_or_default(),
    );
    let (details, path_parts) = if let Some(folder) = folder {
        (Vec::new(), crate::operation_presentation::path_rows(folder))
    } else {
        delete
            .map(|form| (form.details, form.path_parts))
            .unwrap_or_default()
    };
    window.set_details(slint::ModelRc::new(slint::VecModel::from(details)));
    window.set_path_parts(slint::ModelRc::new(slint::VecModel::from(path_parts)));
    match &key {
        Key::Copy(_) => copy_draft_windows::initialize(
            &window,
            main,
            p.copy_draft_links_view(),
            p.copy_preserve_metadata(),
        ),
        Key::Operation(..) => {
            let (label, action) = match main.get_operation_kind() {
                0 => ("New name", "Rename".into()),
                3 => ("Directory name", "Create".into()),
                4 => ("File name", main.get_operation_title()),
                _ => ("", main.get_operation_title()),
            };
            window.set_title_text(main.get_operation_title());
            window.set_kind(main.get_operation_kind());
            window.set_confirm_text(action);
            window.set_body(main.get_operation_body());
            window.set_initial_name(main.get_operation_name());
            window.set_selection_end(main.get_operation_selection_end());
            window.set_has_name(main.get_operation_has_name());
            window.set_input_label(label.into());
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
    let weak = Rc::downgrade(ctx);
    let expected = token.clone();
    window.on_link_chosen(move |token, choice| {
        if token != expected {
            return;
        }
        if let Some(ctx) = weak.upgrade() {
            link_chosen(&ctx, session, choice);
        }
    });
    let weak = Rc::downgrade(ctx);
    let expected = token.clone();
    window.on_browse_destination(move |token, value| {
        if token != expected {
            return;
        }
        let Some(ctx) = weak.upgrade() else { return };
        let (Some(record), Some(state), Some(main)) = (
            current(&ctx, session),
            ctx.state.upgrade(),
            ctx.main.upgrade(),
        ) else {
            return;
        };
        let Key::Copy(token) = &record.key else {
            return;
        };
        if record.cancelling.get() {
            return;
        }
        {
            let mut p = state.borrow_mut();
            if !record.key.current(&p) {
                return;
            }
            record.error.borrow_mut().clear();
            p.browse_copy_destination(
                token,
                &value,
                crate::picker_window::owner(&main),
                ctx.send.as_ref(),
            );
        }
        crate::refresh(&main, &state.borrow());
    });
    let record = Record {
        key,
        session,
        token,
        main: main.as_weak(),
        focus,
        error: RefCell::new(String::new()),
        cancelling: Cell::new(false),
        displayed_pick: Cell::new(None),
        retry: slint::Timer::default(),
    };
    update_warning(&record, main);
    window.invoke_begin();
    Ok(record)
}
/// The form's warning: the presenter's (a Copy/Move draft's error), with the window's own
/// when it says something else. A Copy/Move warning that is only the draft's error about
/// its link choice is shown under that choice.
fn update_warning(record: &Record, main: &MainWindow) {
    let (base, about_links) = match record.key {
        Key::Operation(..) => (main.get_operation_warning(), false),
        Key::Copy(_) => (
            main.get_copy_draft_error(),
            main.get_copy_draft_error_links(),
        ),
    };
    let error = record.error.borrow();
    let (warning, about_links) = if error.is_empty() || base.as_str() == error.as_str() {
        (base, about_links)
    } else if base.is_empty() {
        (error.as_str().into(), false)
    } else {
        (format!("{base}\n\n{error}").into(), false)
    };
    let window = main.global::<OperationPresentation>();
    window.set_warning_about_links(about_links);
    window.set_warning(warning);
}
/// The user chose the Copy/Move link handling (0 copies the files, 1 keeps links): the
/// presenter keeps that choice for the draft, whatever its destination reports later.
fn link_chosen(ctx: &Rc<Context>, session: DialogSession, choice: i32) {
    let (Some(record), Some(state), Some(main)) = (
        current(ctx, session),
        ctx.state.upgrade(),
        ctx.main.upgrade(),
    ) else {
        return;
    };
    let Key::Copy(token) = &record.key else {
        return;
    };
    let Ok(mut p) = state.try_borrow_mut() else {
        return;
    };
    p.choose_copy_draft_links(token, choice == 0);
    copy_draft_windows::refresh_links(
        &main.global::<OperationPresentation>(),
        p.copy_draft_links_view().as_ref(),
    );
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
        Key::Operation(_, token) => {
            let mut p = state.borrow_mut();
            if !record.key.current(&p) {
                return;
            }
            // The dialog also stays open when the presenter asks before replacing an
            // existing file; only a refused send means the queue is busy.
            let refused = Cell::new(false);
            let send = |command: Command| {
                let sent = (ctx.send)(command);
                refused.set(refused.get() || !sent);
                sent
            };
            p.confirm_operation(token, value.to_string(), &send);
            *record.error.borrow_mut() = if refused.get() {
                "The operation queue is busy. Try again.".into()
            } else {
                String::new()
            };
            !record.key.current(&p)
        }
        Key::Copy(token) => {
            match crate::with_owned_commands(&state, &ctx.begin_owned, |p, send| {
                if !record.key.current(p) {
                    return false;
                }
                let window = main.global::<OperationPresentation>();
                let copy_files = window
                    .get_links_visible()
                    .then(|| window.get_link_choice() == 0);
                *record.error.borrow_mut() = copy_draft_windows::confirm(
                    p,
                    token,
                    value.as_str(),
                    window.get_option_checked(),
                    window.get_wait_checked(),
                    copy_files,
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
            if !state
                .try_borrow_mut()
                .is_ok_and(|mut p| p.cancel_copy_destination_picker(ctx.send.as_ref()))
            {
                wait_to_cancel(ctx, &record);
                update_warning(&record, &main);
                return;
            }
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
        Key::Operation(id, _) => {
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
        Key::Operation(id, _) => ctx.failures.record(id, message),
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
