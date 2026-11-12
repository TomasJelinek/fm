//! One Hot Paths manager; Add retains a typed location captured at first opening.
use crate::tool_window_failure::Failures;
use crate::{
    HotPathsWindow, MainWindow, Presenter, ShortcutHints,
    dialog_host::{
        CloseDisposition, DialogHost, DialogModality, DialogOwner, DialogSession, DialogSessions,
    },
    dialog_presentation::{self, DialogThemeTarget},
};
use midnight_core::{Bookmark, BookmarkId, Command, PaneId};
use slint::{ComponentHandle, PlatformError, SharedString};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
struct Record {
    session: DialogSession,
    location: Bookmark,
    window: HotPathsWindow,
    host: DialogHost,
}
struct Context {
    main: slint::Weak<MainWindow>,
    state: std::rc::Weak<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
    owner: DialogOwner,
    sessions: RefCell<DialogSessions>,
    record: RefCell<Option<Rc<Record>>>,
    closing: Rc<Cell<bool>>,
    failures: Failures<u8>,
}
impl DialogThemeTarget for HotPathsWindow {
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
    let closing = Rc::new(Cell::new(false));
    // Reporting presentation failure must not alter bookmarks or backend work.
    let failures = Failures::new(
        main,
        &state,
        &send,
        &closing,
        "Hot Paths window",
        |_, _: u8, _| true,
    );
    let context = Rc::new(Context {
        main: main.as_weak(),
        state: Rc::downgrade(&state),
        send,
        owner: DialogOwner::new(main),
        sessions: RefCell::default(),
        record: RefCell::default(),
        closing,
        failures,
    });
    let ctx = context.clone();
    main.on_open_hot_paths_window(move |pane| {
        let (Some(main), Some(state)) = (ctx.main.upgrade(), ctx.state.upgrade()) else {
            return;
        };
        let pane = match pane {
            0 => PaneId::Left,
            1 => PaneId::Right,
            _ => return,
        };
        if ctx.closing.get() {
            return;
        }
        if let Some(record) = ctx.record.borrow().clone() {
            record.host.activate(record.window.window());
            return;
        }
        if !ctx.owner.is_ready() {
            return;
        }
        let name = if pane == PaneId::Left {
            main.get_left_bookmark_name()
        } else {
            main.get_right_bookmark_name()
        };
        let location = state
            .borrow()
            .capture_bookmark_location(pane, name.to_string());
        let Some(location) = location else {
            state.borrow_mut().desktop.inform(
                "Hot Paths",
                "Wait for Network navigation to finish before saving this location.".into(),
            );
            crate::refresh(&main, &state.borrow());
            return;
        };
        let result = {
            let p = state.borrow();
            open(&ctx, &main, &p, location)
        };
        match result {
            Ok(record) => *ctx.record.borrow_mut() = Some(Rc::new(record)),
            Err(error) => ctx.failures.record(
                0,
                format!("Unable to open Hot Paths: {error}. Open Hot Paths again to retry."),
            ),
        }
    });
    let ctx = context.clone();
    main.on_refresh_hot_paths_window(move || {
        let (Some(main), Some(state)) = (ctx.main.upgrade(), ctx.state.upgrade()) else {
            return;
        };
        if let Some(record) = ctx.record.borrow().clone() {
            update(&record.window, &main, &state.borrow());
        }
    });
    main.on_close_hot_paths_window(move || {
        context.closing.set(true);
        let record = context.record.borrow_mut().take();
        close(record, false);
    });
}
fn update(window: &HotPathsWindow, main: &MainWindow, p: &Presenter) {
    dialog_presentation::apply(window, p);
    window.set_model(main.get_bookmarks());
    window.set_action_icons(main.get_action_icons());
    window
        .global::<ShortcutHints>()
        .set_revision(main.global::<ShortcutHints>().get_revision());
}
fn open(
    ctx: &Rc<Context>,
    main: &MainWindow,
    p: &Presenter,
    location: Bookmark,
) -> Result<Record, PlatformError> {
    main.invoke_drag_invalidate_targets();
    let session = ctx.sessions.borrow_mut().allocate()?;
    let (window, host) = DialogHost::create(
        session,
        ctx.owner.clone(),
        DialogModality::OwnerModal,
        HotPathsWindow::new,
    )?;
    crate::tooltip::install_hot_paths(&window)?;
    let weak = main.as_weak();
    window
        .global::<ShortcutHints>()
        .on_lookup(move |action, all| {
            weak.upgrade().map_or_else(SharedString::default, |main| {
                main.global::<ShortcutHints>().invoke_lookup(action, all)
            })
        });
    window.set_path(location.display_text().into());
    update(&window, main, p);
    let weak = Rc::downgrade(ctx);
    window.on_add_bookmark(move || {
        if let Some(ctx) = weak.upgrade() {
            act(&ctx, session, |p, record, send| {
                p.add_bookmark_location(record.location.clone(), send);
            });
        }
    });
    let weak = Rc::downgrade(ctx);
    window.on_assign_bookmark(move |identity, slot| {
        if let Some(ctx) = weak.upgrade() {
            act(&ctx, session, |p, _, send| {
                if let (Ok(id), Ok(slot)) = (identity.parse::<u64>(), u8::try_from(slot)) {
                    p.set_bookmark_slot(BookmarkId(id), (slot != 0).then_some(slot), send);
                }
            });
        }
    });
    let weak = Rc::downgrade(ctx);
    window.on_remove_bookmark(move |identity| {
        if let Some(ctx) = weak.upgrade() {
            act(&ctx, session, |p, _, send| {
                if let Ok(id) = identity.parse::<u64>() {
                    p.remove_bookmark(BookmarkId(id), send);
                }
            });
        }
    });
    let weak = Rc::downgrade(ctx);
    window.on_request_dismiss(move || {
        if let Some(ctx) = weak.upgrade() {
            let current = ctx
                .record
                .borrow()
                .as_ref()
                .is_some_and(|r| r.session == session && r.host.is_current(session));
            if current {
                let record = ctx.record.borrow_mut().take();
                close(record, true);
            }
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
        session,
        location,
        window,
        host,
    })
}
fn act(
    ctx: &Context,
    session: DialogSession,
    action: impl FnOnce(&mut Presenter, &Record, &dyn Fn(Command) -> bool),
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
    action(&mut state.borrow_mut(), &record, ctx.send.as_ref());
    crate::refresh(&main, &state.borrow());
}
fn close(record: Option<Rc<Record>>, explicit: bool) {
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
#[cfg(test)]
#[path = "hot_path_window_tests.rs"]
mod tests;
