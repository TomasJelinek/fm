//! Modeless job history owns presentation only; closing it never cancels a worker.
use crate::tool_window_failure::Failures;
use crate::{
    JobsView, JobsWindow, MainWindow, Presenter,
    dialog_host::{
        CloseDisposition, DialogHost, DialogModality, DialogOwner, DialogSession, DialogSessions,
    },
    dialog_presentation::{self, DialogThemeTarget},
};
use midnight_core::Command;
use slint::{ComponentHandle, PlatformError, SharedString};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[path = "job_progress_windows.rs"]
mod progress;

pub(crate) type ThemeRevision = (i32, i32, String, i32, i32);
pub(crate) fn theme(p: &Presenter) -> ThemeRevision {
    let values = &p.preferences.values;
    (
        p.preferences.theme_index(),
        p.preferences.density_index(),
        values.font_family.clone(),
        i32::from(values.desktop_font_size),
        i32::from(values.touch_font_size),
    )
}
impl DialogThemeTarget for JobsWindow {
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
struct Record {
    window: JobsWindow,
    host: DialogHost,
    session: DialogSession,
    theme: ThemeRevision,
}
struct Windows {
    sessions: DialogSessions,
    record: Option<Record>,
    closing: bool,
}
struct Context {
    main: slint::Weak<MainWindow>,
    state: std::rc::Weak<RefCell<Presenter>>,
    owner: DialogOwner,
    windows: RefCell<Windows>,
    closing: Rc<Cell<bool>>,
    failures: Failures<u8>,
    progress: Rc<progress::Windows>,
}

pub(crate) fn install(
    main: &MainWindow,
    state: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
) {
    let closing = Rc::new(Cell::new(false));
    // This reporter only shows a bounded owned notice. Job/decision authority
    // remains pending; reporting must not submit a cancellation or retry.
    let failures = Failures::new(
        main,
        &state,
        &send,
        &closing,
        "Jobs window",
        |_, _: u8, _| true,
    );
    let ctx = Rc::new(Context {
        progress: progress::Windows::install(main, &state, &send),
        main: main.as_weak(),
        state: Rc::downgrade(&state),
        closing,
        failures,
        owner: DialogOwner::new(main),
        windows: RefCell::new(Windows {
            sessions: DialogSessions::default(),
            record: None,
            closing: false,
        }),
    });
    let context = ctx.clone();
    main.on_refresh_jobs_window(move |activate| {
        let (Some(main), Some(state)) = (context.main.upgrade(), context.state.upgrade()) else {
            return;
        };
        if let Err(error) = synchronize(&context, &main, &state.borrow(), activate) {
            // A failed show does not mutate jobs or retry automatically on every progress tick.
            main.invoke_close_operations();
            context.failures.record(
                0,
                format!("Unable to open Jobs: {error}. Open Jobs again to retry."),
            );
        }
    });
    main.on_close_jobs_window(move || {
        ctx.closing.set(true);
        ctx.progress.close_all();
        let record = {
            let mut windows = ctx.windows.borrow_mut();
            windows.closing = true;
            windows.record.take()
        };
        close(record, false);
    });
}
fn synchronize(
    ctx: &Rc<Context>,
    main: &MainWindow,
    p: &Presenter,
    activate: bool,
) -> Result<(), PlatformError> {
    ctx.progress.synchronize(main, p);
    if ctx.windows.borrow().closing || !main.get_operations_visible() {
        let record = ctx.windows.borrow_mut().record.take();
        close(record, false);
        return Ok(());
    }
    if let Some(record) = ctx.windows.borrow_mut().record.as_mut() {
        if record.theme != theme(p) {
            dialog_presentation::apply(&record.window, p);
            record.theme = theme(p);
        }
        record.window.set_rows(main.get_operations());
        record
            .window
            .set_requested_identity(main.get_command_preview_token());
        if activate {
            record.host.activate(record.window.window());
        }
        return Ok(());
    }
    if !ctx.owner.is_ready() {
        return Ok(());
    }
    let session = ctx.windows.borrow_mut().sessions.allocate()?;
    let (window, host) = DialogHost::create(
        session,
        ctx.owner.clone(),
        DialogModality::Modeless,
        JobsWindow::new,
    )?;
    crate::tooltip::install_jobs(&window)?;
    dialog_presentation::apply(&window, p);
    window
        .global::<JobsView>()
        .on_find(crate::job_presentation::find);
    window
        .global::<JobsView>()
        .on_has_finished(crate::job_presentation::has_finished);
    window.set_rows(main.get_operations());
    window.set_requested_identity(main.get_command_preview_token());
    macro_rules! action {
        ($callback:ident, $invoke:ident) => {{
            let weak = Rc::downgrade(ctx);
            window.$callback(move |identity| {
                let Some(ctx) = weak.upgrade() else {
                    return;
                };
                if !current(&ctx, session) {
                    return;
                }
                if let Some(main) = ctx.main.upgrade() {
                    main.$invoke(identity);
                }
            });
        }};
    }
    action!(on_open_progress, invoke_open_job_progress);
    action!(on_cancel, invoke_cancel_job);
    action!(on_resolve, invoke_resolve_copy);
    action!(on_copy_details, invoke_copy_job_details);
    action!(on_clear_finished, invoke_clear_finished_jobs);
    let weak = Rc::downgrade(ctx);
    window.on_request_dismiss(move || {
        if let Some(ctx) = weak.upgrade() {
            dismiss(&ctx, session);
        }
    });
    let weak = window.as_weak();
    window.window().on_close_requested(move || {
        if let Some(window) = weak.upgrade() {
            window.invoke_request_dismiss();
        }
        slint::CloseRequestResponse::KeepWindowShown
    });
    host.show(window.window())?;
    ctx.windows.borrow_mut().record = Some(Record {
        window,
        host,
        session,
        theme: theme(p),
    });
    Ok(())
}
fn current(ctx: &Context, session: DialogSession) -> bool {
    let windows = ctx.windows.borrow();
    !windows.closing
        && windows
            .record
            .as_ref()
            .is_some_and(|r| r.session == session && r.host.is_current(session))
}
fn dismiss(ctx: &Context, session: DialogSession) {
    if !current(ctx, session) {
        return;
    }
    let record = ctx.windows.borrow_mut().record.take();
    close(record, true);
    if let Some(main) = ctx.main.upgrade() {
        main.invoke_close_operations();
    }
}
fn close(record: Option<Record>, explicit: bool) {
    if let Some(record) = record {
        if explicit {
            record
                .host
                .request_close(record.window.window(), || CloseDisposition::Hide);
        } else {
            record.host.close(record.window.window());
        }
    }
}
