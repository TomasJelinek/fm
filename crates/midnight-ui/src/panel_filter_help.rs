//! Filter help belongs to the active modal filter, not to its disabled main owner.
use super::*;
use crate::{HelpWindow, ShortcutHints};

pub(super) struct Record {
    pub(super) window: HelpWindow,
    host: DialogHost,
    session: DialogSession,
}
fn current(ctx: &Context, session: DialogSession) -> Option<Rc<super::Record>> {
    if ctx.closing.get() {
        return None;
    }
    ctx.record
        .borrow()
        .as_ref()
        .filter(|record| record.session == session && record.host.is_current(session))
        .cloned()
}
pub(super) fn show(ctx: &Rc<Context>, session: DialogSession) {
    let (Some(record), Some(state), Some(main)) = (
        current(ctx, session),
        ctx.state.upgrade(),
        ctx.main.upgrade(),
    ) else {
        return;
    };
    let p = state.borrow();
    if !p.filter_dialog.open || p.filter_dialog.generation != record.generation {
        return;
    }
    let result = open(ctx, &record, &main, &p);
    drop(p);
    if let Err(error) = result {
        state.borrow_mut().filter_dialog.error = format!("Unable to open filter help: {error}");
        crate::refresh(&main, &state.borrow());
    }
}
fn open(
    ctx: &Rc<Context>,
    filter: &Rc<super::Record>,
    main: &MainWindow,
    p: &Presenter,
) -> Result<(), PlatformError> {
    if let Some(record) = filter.help.borrow().as_ref() {
        record.host.show(record.window.window())?;
        record.host.activate(record.window.window());
        return Ok(());
    }
    let session = ctx.sessions.borrow_mut().allocate()?;
    let (window, host) = DialogHost::create(
        session,
        DialogOwner::new(&filter.window),
        DialogModality::OwnerModal,
        HelpWindow::new,
    )?;
    crate::tooltip::install_help(&window)?;
    dialog_presentation::apply(&window, p);
    window.set_topic(1);
    window.set_close_icon(main.get_action_icons().close);
    window.set_shortcuts(main.global::<ShortcutHints>().invoke_help());
    let parent_session = filter.session;
    let weak = Rc::downgrade(ctx);
    window.on_request_dismiss(move || {
        let Some(ctx) = weak.upgrade() else {
            return;
        };
        let Some(filter) = current(&ctx, parent_session) else {
            return;
        };
        let valid = filter
            .help
            .borrow()
            .as_ref()
            .is_some_and(|record| record.session == session && record.host.is_current(session));
        if valid {
            close(&filter, true);
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
    *filter.help.borrow_mut() = Some(Record {
        window,
        host,
        session,
    });
    Ok(())
}
pub(super) fn close(filter: &super::Record, explicit: bool) {
    let record = filter.help.borrow_mut().take();
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
