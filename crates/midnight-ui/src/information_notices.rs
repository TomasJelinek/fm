//! Generation-bound notices live in the main client area, never a native child window.
use crate::{
    MainWindow, NoticePresentation,
    dialog_focus::{FocusReturn, restore_after_popup},
};
use slint::ComponentHandle;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

pub(super) struct Context {
    main: slint::Weak<MainWindow>,
    requested: Cell<Option<i32>>,
    shown: Cell<Option<i32>>,
    closing: Cell<bool>,
    focus: RefCell<Option<FocusReturn>>,
}

pub(super) fn install(main: &MainWindow) -> Rc<Context> {
    let context = Rc::new(Context {
        main: main.as_weak(),
        requested: Cell::new(None),
        shown: Cell::new(None),
        closing: Cell::new(false),
        focus: RefCell::new(None),
    });
    let weak = Rc::downgrade(&context);
    main.on_open_notice_window(move || {
        let Some(context) = weak.upgrade() else {
            return;
        };
        let Some(main) = context.main.upgrade() else {
            return;
        };
        if context.closing.get() || main.get_message_generation() == 0 {
            return;
        }
        context.requested.set(Some(main.get_message_generation()));
        refresh(&context, &main);
    });
    let presentation = main.global::<NoticePresentation>();
    let weak = Rc::downgrade(&context);
    presentation.on_copy_details(move |token| {
        let Some(context) = weak.upgrade() else {
            return;
        };
        let Some(main) = context.main.upgrade() else {
            return;
        };
        if current(&context, &main, token.as_str()) {
            main.invoke_copy_message();
        }
    });
    let weak = Rc::downgrade(&context);
    presentation.on_dismiss(move |token| {
        let Some(context) = weak.upgrade() else {
            return;
        };
        let Some(main) = context.main.upgrade() else {
            return;
        };
        if current(&context, &main, token.as_str()) {
            dismiss(&context, &main);
        }
    });
    let weak = Rc::downgrade(&context);
    presentation.on_authorize_retry(move |token| {
        let Some(context) = weak.upgrade() else {
            return;
        };
        let Some(main) = context.main.upgrade() else {
            return;
        };
        if current(&context, &main, token.as_str()) && main.get_message_can_authorize() {
            let generation = main.get_message_generation();
            // Retire this presentation before a reentrant/new diagnostic can arrive.
            retire(&context, &main);
            main.invoke_authorize_message(generation);
            main.invoke_diagnostic_notice_closed(generation);
        }
    });
    context
}

pub(super) fn refresh(context: &Rc<Context>, main: &MainWindow) {
    if context.closing.get() || context.requested.get().is_none() || !main.window().is_visible() {
        return;
    }
    let generation = main.get_message_generation();
    if generation == 0 {
        retire(context, main);
        return;
    }
    let presentation = main.global::<NoticePresentation>();
    if context.shown.get().is_none() {
        *context.focus.borrow_mut() = Some(FocusReturn::capture(main.window()));
        main.invoke_drag_invalidate_targets();
    }
    context.requested.set(Some(generation));
    presentation.set_title(main.get_message_title());
    presentation.set_body(main.get_message_body());
    presentation.set_folder_path(main.get_message_path());
    presentation.set_details(main.get_message_details());
    presentation.set_can_authorize(main.get_message_can_authorize());
    presentation.set_error(main.get_message_error());
    presentation.set_generation(generation);
    context.shown.set(Some(generation));
    presentation.set_token(generation.to_string().into());
}

fn current(context: &Context, main: &MainWindow, token: &str) -> bool {
    !context.closing.get()
        && context.shown.get() == Some(main.get_message_generation())
        && context.requested.get() == context.shown.get()
        && main.global::<NoticePresentation>().get_token().as_str() == token
        && !token.is_empty()
}

fn dismiss(context: &Rc<Context>, main: &MainWindow) {
    let generation = main.get_message_generation();
    retire(context, main);
    main.invoke_diagnostic_notice_closed(generation);
}

fn retire(context: &Rc<Context>, main: &MainWindow) {
    context.requested.set(None);
    context.shown.set(None);
    main.global::<NoticePresentation>().set_token("".into());
    let focus = context.focus.borrow_mut().take();
    let weak = Rc::downgrade(context);
    restore_after_popup(main, focus, move || {
        weak.upgrade()
            .is_some_and(|context| !context.closing.get() && context.requested.get().is_none())
    });
}

pub(super) fn close(context: &Context, main: &MainWindow) {
    context.closing.set(true);
    context.requested.set(None);
    context.shown.set(None);
    context.focus.borrow_mut().take();
    main.global::<NoticePresentation>().set_token("".into());
}
