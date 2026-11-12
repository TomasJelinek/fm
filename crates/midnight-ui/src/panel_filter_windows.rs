//! Embedded filter presentation; Presenter retains captured navigation and validation.
use crate::{
    MainWindow, PanelFilterPresentation, PanelFilterWindow, Presenter,
    dialog_focus::{FocusReturn, restore_after_popup},
    dialog_presentation::DialogThemeTarget,
};
use midnight_core::{Command, FilterMode, PanelFilter};
use slint::{ComponentHandle, SharedString};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[path = "panel_filter_help.rs"]
mod help;

struct Context {
    main: slint::Weak<MainWindow>,
    state: std::rc::Weak<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
    generation: Cell<Option<u64>>,
    focus: RefCell<Option<FocusReturn>>,
    help: RefCell<Option<help::Record>>,
    next_help: Cell<u64>,
    closing: Cell<bool>,
}

// Standalone wrappers remain useful to exercise the form at native-window sizes.
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
        generation: Cell::new(None),
        focus: RefCell::default(),
        help: RefCell::default(),
        next_help: Cell::new(0),
        closing: Cell::new(false),
    });
    let ctx = context.clone();
    main.on_refresh_panel_filter_window(move || {
        let (Some(main), Some(state)) = (ctx.main.upgrade(), ctx.state.upgrade()) else {
            return;
        };
        synchronize(&ctx, &main, &state.borrow());
    });
    let ctx = context.clone();
    main.global::<PanelFilterPresentation>().on_apply(
        move |token, enabled, mode, expression, case_sensitive| {
            act(&ctx, &token, move |p, send| {
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
        },
    );
    let ctx = context.clone();
    main.global::<PanelFilterPresentation>()
        .on_clear(move |token| {
            act(&ctx, &token, |p, send| p.clear_captured_panel_filter(send));
        });
    let ctx = context.clone();
    main.global::<PanelFilterPresentation>()
        .on_dismiss(move |token| {
            act(&ctx, &token, |p, _| p.cancel_panel_filter());
        });
    let ctx = context.clone();
    main.global::<PanelFilterPresentation>()
        .on_show_help(move |token| help::show(&ctx, &token));
    let ctx = context.clone();
    main.global::<PanelFilterPresentation>()
        .on_dismiss_help(move |token, help_token| help::dismiss(&ctx, &token, &help_token));
    main.on_close_panel_filter_window(move || {
        context.closing.set(true);
        if let Some(main) = context.main.upgrade() {
            retire(&context, &main);
        }
        if let Some(state) = context.state.upgrade() {
            state.borrow_mut().cancel_panel_filter();
        }
    });
}

fn synchronize(ctx: &Rc<Context>, main: &MainWindow, p: &Presenter) {
    if ctx.closing.get() || !p.filter_dialog.open {
        retire(ctx, main);
        return;
    }
    let generation = p.filter_dialog.generation;
    let presentation = main.global::<PanelFilterPresentation>();
    presentation.set_model(main.get_panel_filter_view());
    presentation.set_close_icon(main.get_action_icons().close);
    if ctx.generation.get() == Some(generation) {
        // The existing controls own edits; status/theme changes never call begin.
        return;
    }
    if ctx.focus.borrow().is_none() {
        *ctx.focus.borrow_mut() = Some(FocusReturn::capture(main.window()));
    }
    ctx.help.borrow_mut().take();
    presentation.set_help_token(SharedString::default());
    main.invoke_drag_invalidate_targets();
    ctx.generation.set(Some(generation));
    presentation.set_token(generation.to_string().into());
}

fn current(ctx: &Context, token: &str) -> Option<u64> {
    if ctx.closing.get() {
        return None;
    }
    let generation = token.parse::<u64>().ok()?;
    (ctx.generation.get() == Some(generation)).then_some(generation)
}

fn act(
    ctx: &Rc<Context>,
    token: &str,
    action: impl FnOnce(&mut Presenter, &dyn Fn(Command) -> bool),
) {
    let (Some(generation), Some(state), Some(main)) =
        (current(ctx, token), ctx.state.upgrade(), ctx.main.upgrade())
    else {
        return;
    };
    if ctx.help.borrow().is_some() {
        return;
    }
    let closed = {
        let mut p = state.borrow_mut();
        if !p.filter_dialog.open || p.filter_dialog.generation != generation {
            return;
        }
        action(&mut p, ctx.send.as_ref());
        !p.filter_dialog.open
    };
    if closed {
        retire(ctx, &main);
    }
    crate::refresh(&main, &state.borrow());
}

fn retire(ctx: &Rc<Context>, main: &MainWindow) {
    let generation = ctx.generation.replace(None);
    ctx.help.borrow_mut().take();
    let presentation = main.global::<PanelFilterPresentation>();
    presentation.set_token(SharedString::default());
    presentation.set_help_token(SharedString::default());
    let Some(focus) = ctx.focus.borrow_mut().take() else {
        return;
    };
    let weak = Rc::downgrade(ctx);
    restore_after_popup(main, Some(focus), move || {
        weak.upgrade().is_some_and(|ctx| {
            !ctx.closing.get()
                && ctx.generation.get().is_none()
                && ctx.state.upgrade().is_some_and(|state| {
                    state.try_borrow().is_ok_and(|p| {
                        !p.filter_dialog.open && Some(p.filter_dialog.generation) == generation
                    })
                })
        })
    });
}

#[cfg(test)]
#[path = "panel_filter_window_tests.rs"]
mod tests;
