//! Help is a nested embedded popup tied to one live filter occurrence.
use super::*;

pub(super) struct Record {
    generation: u64,
    token: u64,
    focus: FocusReturn,
}

pub(super) fn show(ctx: &Rc<Context>, token: &str) {
    let (Some(generation), Some(state), Some(main)) =
        (current(ctx, token), ctx.state.upgrade(), ctx.main.upgrade())
    else {
        return;
    };
    if ctx.help.borrow().is_some() {
        return;
    }
    let p = state.borrow();
    if !p.filter_dialog.open || p.filter_dialog.generation != generation {
        return;
    }
    drop(p);
    let Some(help) = ctx.next_help.get().checked_add(1) else {
        state.borrow_mut().filter_dialog.error = "Filter help identities are exhausted.".into();
        crate::refresh(&main, &state.borrow());
        return;
    };
    ctx.next_help.set(help);
    *ctx.help.borrow_mut() = Some(Record {
        generation,
        token: help,
        focus: FocusReturn::capture(main.window()),
    });
    main.global::<PanelFilterPresentation>()
        .set_help_token(help.to_string().into());
}

pub(super) fn dismiss(ctx: &Rc<Context>, token: &str, help_token: &str) {
    let (Some(generation), Ok(help), Some(main)) = (
        current(ctx, token),
        help_token.parse::<u64>(),
        ctx.main.upgrade(),
    ) else {
        return;
    };
    if !ctx
        .help
        .borrow()
        .as_ref()
        .is_some_and(|record| record.generation == generation && record.token == help)
    {
        return;
    }
    let Some(record) = ctx.help.borrow_mut().take() else {
        return;
    };
    main.global::<PanelFilterPresentation>()
        .set_help_token(SharedString::default());
    let weak = Rc::downgrade(ctx);
    slint::Timer::single_shot(std::time::Duration::ZERO, move || {
        let Some(ctx) = weak.upgrade() else {
            return;
        };
        if ctx.closing.get()
            || ctx.generation.get() != Some(generation)
            || ctx.help.borrow().is_some()
            || ctx.next_help.get() != help
        {
            return;
        }
        if let Some(main) = ctx.main.upgrade() {
            // The shared top-popup guard refuses focus covered by another modal.
            record.focus.restore(main.window());
        }
    });
}
