//! Embedded Hot Paths manager; Add retains the location captured at first opening.
use crate::{
    HotPathsPresentation, HotPathsWindow, MainWindow, Presenter,
    dialog_focus::{FocusReturn, restore_after_popup},
    dialog_presentation::DialogThemeTarget,
};
use midnight_core::{Bookmark, BookmarkId, Command, PaneId};
use slint::{ComponentHandle, SharedString};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[derive(Clone)]
struct Record {
    token: u64,
    location: Bookmark,
    focus: FocusReturn,
}

struct Context {
    main: slint::Weak<MainWindow>,
    state: std::rc::Weak<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
    next: Cell<u64>,
    record: RefCell<Option<Record>>,
    closing: Cell<bool>,
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
    let context = Rc::new(Context {
        main: main.as_weak(),
        state: Rc::downgrade(&state),
        send,
        next: Cell::new(0),
        record: RefCell::default(),
        closing: Cell::new(false),
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
        if ctx.closing.get() || ctx.record.borrow().is_some() {
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
        let Some(token) = ctx.next.get().checked_add(1) else {
            state
                .borrow_mut()
                .desktop
                .inform("Hot Paths", "Hot Paths identities are exhausted.".into());
            crate::refresh(&main, &state.borrow());
            return;
        };
        ctx.next.set(token);
        let presentation = main.global::<HotPathsPresentation>();
        presentation.set_path(location.display_text().into());
        *ctx.record.borrow_mut() = Some(Record {
            token,
            location,
            focus: FocusReturn::capture(main.window()),
        });
        update(&main);
        main.invoke_drag_invalidate_targets();
        presentation.set_token(token.to_string().into());
    });
    let ctx = context.clone();
    main.on_refresh_hot_paths_window(move || {
        if !ctx.closing.get()
            && ctx.record.borrow().is_some()
            && let Some(main) = ctx.main.upgrade()
        {
            update(&main);
        }
    });
    let ctx = context.clone();
    main.global::<HotPathsPresentation>()
        .on_add_bookmark(move |token| {
            act(&ctx, &token, |p, record, send| {
                p.add_bookmark_location(record.location.clone(), send);
            });
        });
    let ctx = context.clone();
    main.global::<HotPathsPresentation>()
        .on_assign_bookmark(move |token, identity, slot| {
            act(&ctx, &token, |p, _, send| {
                if let (Ok(id), Ok(slot)) = (identity.parse::<u64>(), u8::try_from(slot)) {
                    p.set_bookmark_slot(BookmarkId(id), (slot != 0).then_some(slot), send);
                }
            });
        });
    let ctx = context.clone();
    main.global::<HotPathsPresentation>()
        .on_remove_bookmark(move |token, identity| {
            act(&ctx, &token, |p, _, send| {
                if let Ok(id) = identity.parse::<u64>() {
                    p.remove_bookmark(BookmarkId(id), send);
                }
            });
        });
    let ctx = context.clone();
    main.global::<HotPathsPresentation>()
        .on_dismiss(move |token| {
            if current(&ctx, &token).is_some()
                && let Some(main) = ctx.main.upgrade()
            {
                retire(&ctx, &main);
            }
        });
    main.on_close_hot_paths_window(move || {
        context.closing.set(true);
        if let Some(main) = context.main.upgrade() {
            retire(&context, &main);
        }
    });
}

fn update(main: &MainWindow) {
    let presentation = main.global::<HotPathsPresentation>();
    presentation.set_model(main.get_bookmarks());
    presentation.set_action_icons(main.get_action_icons());
}

fn current(ctx: &Context, token: &str) -> Option<Record> {
    if ctx.closing.get() {
        return None;
    }
    let token = token.parse::<u64>().ok()?;
    ctx.record
        .borrow()
        .as_ref()
        .filter(|record| record.token == token)
        .cloned()
}

fn act(
    ctx: &Context,
    token: &str,
    action: impl FnOnce(&mut Presenter, &Record, &dyn Fn(Command) -> bool),
) {
    let (Some(record), Some(state), Some(main)) =
        (current(ctx, token), ctx.state.upgrade(), ctx.main.upgrade())
    else {
        return;
    };
    action(&mut state.borrow_mut(), &record, ctx.send.as_ref());
    crate::refresh(&main, &state.borrow());
}

fn retire(ctx: &Rc<Context>, main: &MainWindow) {
    let record = ctx.record.borrow_mut().take();
    main.global::<HotPathsPresentation>()
        .set_token(SharedString::default());
    let Some(record) = record else {
        return;
    };
    let weak = Rc::downgrade(ctx);
    restore_after_popup(main, Some(record.focus), move || {
        weak.upgrade().is_some_and(|ctx| {
            !ctx.closing.get() && ctx.record.borrow().is_none() && ctx.next.get() == record.token
        })
    });
}

#[cfg(test)]
#[path = "hot_path_window_tests.rs"]
mod tests;
