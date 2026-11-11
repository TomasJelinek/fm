//! Native panel filter lifetime; captured navigation remains owned by Presenter.
use crate::{
    MainWindow, PanelFilterWindow, Presenter,
    dialog_host::{
        CloseDisposition, DialogHost, DialogModality, DialogOwner, DialogSession, DialogSessions,
    },
    dialog_presentation::{self, DialogThemeTarget},
};
use midnight_core::{Command, FilterMode, PanelFilter};
use slint::{ComponentHandle, PlatformError, SharedString};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

#[path = "panel_filter_help.rs"]
mod help;

struct Record {
    generation: u64,
    session: DialogSession,
    window: PanelFilterWindow,
    host: DialogHost,
    help: RefCell<Option<help::Record>>,
}
struct Context {
    main: slint::Weak<MainWindow>,
    state: std::rc::Weak<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
    owner: DialogOwner,
    sessions: RefCell<DialogSessions>,
    record: RefCell<Option<Rc<Record>>>,
    failed: Cell<Option<u64>>,
    closing: Cell<bool>,
}
impl DialogThemeTarget for PanelFilterWindow {
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
pub(crate) fn install(
    main: &MainWindow,
    state: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
) {
    let context = Rc::new(Context {
        main: main.as_weak(),
        state: Rc::downgrade(&state),
        send,
        owner: DialogOwner::new(main),
        sessions: RefCell::default(),
        record: RefCell::default(),
        failed: Cell::new(None),
        closing: Cell::new(false),
    });
    let ctx = context.clone();
    main.on_refresh_panel_filter_window(move || {
        let (Some(main), Some(state)) = (ctx.main.upgrade(), ctx.state.upgrade()) else {
            return;
        };
        synchronize(&ctx, &main, &state.borrow());
    });
    main.on_close_panel_filter_window(move || {
        context.closing.set(true);
        let record = context.record.borrow_mut().take();
        close(record, false);
        if let Some(state) = context.state.upgrade() {
            state.borrow_mut().cancel_panel_filter();
        }
    });
}
fn synchronize(ctx: &Rc<Context>, main: &MainWindow, p: &Presenter) {
    let old = {
        let mut record = ctx.record.borrow_mut();
        if record.as_ref().is_some_and(|r| {
            !p.filter_dialog.open || r.generation != p.filter_dialog.generation || ctx.closing.get()
        }) {
            record.take()
        } else {
            None
        }
    };
    close(old, false);
    if ctx.closing.get() || !p.filter_dialog.open {
        return;
    }
    if let Some(record) = ctx.record.borrow().clone() {
        // Updating status/theme never calls begin or rewrites the edited controls.
        record.window.set_model(main.get_panel_filter_view());
        dialog_presentation::apply(&record.window, p);
        if let Some(help) = record.help.borrow().as_ref() {
            dialog_presentation::apply(&help.window, p);
        }
        return;
    }
    let generation = p.filter_dialog.generation;
    if ctx.failed.get() == Some(generation) || !ctx.owner.is_ready() {
        return;
    }
    match open(ctx, main, p) {
        Ok(record) => *ctx.record.borrow_mut() = Some(Rc::new(record)),
        Err(error) => {
            ctx.failed.set(Some(generation));
            release_failed(ctx, generation, error.to_string(), Duration::ZERO);
        }
    }
}
fn open(ctx: &Rc<Context>, main: &MainWindow, p: &Presenter) -> Result<Record, PlatformError> {
    main.invoke_drag_invalidate_targets();
    let session = ctx.sessions.borrow_mut().allocate()?;
    let (window, host) = DialogHost::create(
        session,
        ctx.owner.clone(),
        DialogModality::OwnerModal,
        PanelFilterWindow::new,
    )?;
    let weak = window.as_weak();
    DialogOwner::new(&window).set_input_gate(move |blocked| {
        if let Some(window) = weak.upgrade() {
            window.set_dialog_input_blocked(blocked);
        }
    });
    crate::tooltip::install_panel_filter(&window)?;
    dialog_presentation::apply(&window, p);
    window.set_model(main.get_panel_filter_view());
    let weak = Rc::downgrade(ctx);
    window.on_apply(move |enabled, mode, expression, case_sensitive| {
        if let Some(ctx) = weak.upgrade() {
            act(&ctx, session, move |p, send| {
                let mode = match mode {
                    0 => FilterMode::Masks,
                    1 => FilterMode::Regex,
                    2 => FilterMode::Expression,
                    _ => {
                        p.filter_dialog.error = "Choose Masks, Regex or Expression.".into();
                        return;
                    }
                };
                p.submit_panel_filter(
                    p.filter_dialog.pane,
                    PanelFilter {
                        enabled,
                        mode,
                        expression: expression.to_string(),
                        case_sensitive,
                    },
                    send,
                );
            });
        }
    });
    let weak = Rc::downgrade(ctx);
    window.on_clear(move || {
        if let Some(ctx) = weak.upgrade() {
            act(&ctx, session, |p, send| p.clear_captured_panel_filter(send));
        }
    });
    let weak = Rc::downgrade(ctx);
    window.on_request_dismiss(move || {
        if let Some(ctx) = weak.upgrade() {
            act(&ctx, session, |p, _| p.cancel_panel_filter());
        }
    });
    let weak = Rc::downgrade(ctx);
    window.on_help(move || {
        if let Some(ctx) = weak.upgrade() {
            help::show(&ctx, session);
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
    window.invoke_begin();
    Ok(Record {
        generation: p.filter_dialog.generation,
        session,
        window,
        host,
        help: RefCell::default(),
    })
}
fn act(
    ctx: &Rc<Context>,
    session: DialogSession,
    action: impl FnOnce(&mut Presenter, &dyn Fn(Command) -> bool),
) {
    let record = ctx
        .record
        .borrow()
        .as_ref()
        .filter(|r| r.session == session && r.host.is_current(session))
        .cloned();
    let (Some(record), Some(state), Some(main)) = (record, ctx.state.upgrade(), ctx.main.upgrade())
    else {
        return;
    };
    if ctx.closing.get() {
        return;
    }
    let closed = {
        let mut p = state.borrow_mut();
        if !p.filter_dialog.open || p.filter_dialog.generation != record.generation {
            return;
        }
        action(&mut p, ctx.send.as_ref());
        !p.filter_dialog.open
    };
    if closed {
        let record = ctx.record.borrow_mut().take();
        close(record, true);
    }
    crate::refresh(&main, &state.borrow());
}
fn close(record: Option<Rc<Record>>, explicit: bool) {
    if let Some(record) = record {
        help::close(&record, false);
        if explicit {
            record
                .host
                .request_close(record.window.window(), || CloseDisposition::Hide);
        } else {
            record.host.close(record.window.window());
        }
    }
}
fn release_failed(ctx: &Rc<Context>, generation: u64, message: String, delay: Duration) {
    let weak = Rc::downgrade(ctx);
    slint::Timer::single_shot(delay, move || {
        let Some(ctx) = weak.upgrade() else {
            return;
        };
        let (Some(state), Some(main)) = (ctx.state.upgrade(), ctx.main.upgrade()) else {
            return;
        };
        if ctx.closing.get() || ctx.failed.get() != Some(generation) {
            return;
        }
        let Ok(mut p) = state.try_borrow_mut() else {
            release_failed(&ctx, generation, message, Duration::from_millis(250));
            return;
        };
        if p.filter_dialog.open && p.filter_dialog.generation == generation {
            p.cancel_panel_filter();
            p.desktop.report("Panel filter window", message);
        }
        drop(p);
        crate::refresh(&main, &state.borrow());
    });
}
#[cfg(test)]
#[path = "panel_filter_window_tests.rs"]
mod tests;
