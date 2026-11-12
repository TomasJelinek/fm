//! Exact-token modeless prompts. Deferral and cancellation are separate actions.
use crate::tool_window_failure::Failures;
use crate::{
    DecisionWindow, MainWindow, Presenter,
    dialog_host::{
        CloseDisposition, DialogHost, DialogModality, DialogOwner, DialogSession, DialogSessions,
    },
    dialog_presentation::{self, DialogThemeTarget},
    jobs_windows::{ThemeRevision, theme},
};
use midnight_core::Command;
use slint::{ComponentHandle, PlatformError, SharedString};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[derive(Clone, PartialEq, Eq)]
struct Key {
    metadata: bool,
    token: String,
}
impl Key {
    fn index(&self) -> usize {
        usize::from(self.metadata)
    }
    fn current(&self, p: &Presenter) -> bool {
        if p.operations.closing {
            return false;
        }
        if self.metadata {
            p.operations
                .metadata_conflicts
                .iter()
                .any(|c| self.token == format!("metadata:{}:{}", c.id.0, c.token))
        } else {
            if let Some((id, version)) = p.active_rename_decision() {
                return self.token == format!("rename:{}:{}", id.0, version);
            }
            let active = p.active_copy_decision();
            p.copies.conflicts.iter().any(|c| {
                active == Some((c.id, c.version))
                    && self.token == format!("copy:{}:{}", c.id.0, c.version)
            }) || p.copies.failures.iter().any(|c| {
                active == Some((c.id, c.version))
                    && self.token == format!("copy-error:{}:{}", c.id.0, c.version)
            })
        }
    }
}
struct Record {
    key: Key,
    session: DialogSession,
    window: DecisionWindow,
    host: DialogHost,
    theme: RefCell<ThemeRevision>,
}
#[derive(Default)]
struct Windows {
    sessions: DialogSessions,
    records: [Option<Rc<Record>>; 2],
    wanted: [Option<Key>; 2],
    failed: [Option<Key>; 2],
    closing: bool,
}
struct Context {
    main: slint::Weak<MainWindow>,
    state: std::rc::Weak<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
    owner: DialogOwner,
    windows: RefCell<Windows>,
    closing: Rc<Cell<bool>>,
    failures: Failures<u8>,
}
impl DialogThemeTarget for DecisionWindow {
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
    // This reporter only shows a bounded owned notice. Job/decision authority
    // remains pending; reporting must not submit a cancellation or retry.
    let failures = Failures::new(
        main,
        &state,
        &send,
        &closing,
        "Operation decision",
        |_, _: u8, _| true,
    );
    let ctx = Rc::new(Context {
        main: main.as_weak(),
        state: Rc::downgrade(&state),
        closing,
        failures,
        send,
        owner: DialogOwner::new(main),
        windows: RefCell::new(Windows::default()),
    });
    let context = ctx.clone();
    main.on_open_decision_window(move |metadata, explicit| {
        let (Some(main), Some(state)) = (context.main.upgrade(), context.state.upgrade()) else {
            return;
        };
        let key = Key {
            metadata,
            token: if metadata {
                main.get_metadata_conflict_token()
            } else {
                main.get_copy_token()
            }
            .to_string(),
        };
        if key.token.is_empty() {
            return;
        }
        let index = key.index();
        {
            let mut windows = context.windows.borrow_mut();
            if windows.closing {
                return;
            }
            if explicit {
                windows.failed[index] = None;
            }
            windows.wanted[index] = Some(key);
        }
        // A request may be projected while its caller still mutates Presenter.
        // Keep it pending until the existing end-of-refresh hook can realize it.
        let Ok(p) = state.try_borrow() else {
            return;
        };
        synchronize(&context, &main, &p);
        if explicit && let Some(record) = context.windows.borrow().records[index].as_ref() {
            record.host.activate(record.window.window());
        }
    });
    let context = ctx.clone();
    main.on_refresh_decision_windows(move || {
        let (Some(main), Some(state)) = (context.main.upgrade(), context.state.upgrade()) else {
            return;
        };
        synchronize(&context, &main, &state.borrow());
    });
    let context = ctx.clone();
    main.on_close_decision_window(move |metadata| {
        let index = usize::from(metadata);
        let record = {
            let mut windows = context.windows.borrow_mut();
            windows.wanted[index] = None;
            windows.records[index].take()
        };
        close(record, false);
    });
    main.on_close_decision_windows(move || {
        ctx.closing.set(true);
        let records = {
            let mut windows = ctx.windows.borrow_mut();
            windows.closing = true;
            windows.wanted = Default::default();
            std::mem::take(&mut windows.records)
        };
        for record in records {
            close(record, false);
        }
    });
}
fn synchronize(ctx: &Rc<Context>, main: &MainWindow, p: &Presenter) {
    for index in 0..2 {
        let stale = {
            let mut windows = ctx.windows.borrow_mut();
            if windows.closing
                || windows.records[index].as_ref().is_some_and(|record| {
                    !record.key.current(p)
                        || windows.wanted[index]
                            .as_ref()
                            .is_some_and(|wanted| wanted != &record.key && wanted.current(p))
                })
            {
                windows.records[index].take()
            } else {
                None
            }
        };
        close(stale, false);
        if let Some(record) = ctx.windows.borrow().records[index].clone() {
            let revision = theme(p);
            if *record.theme.borrow() != revision {
                dialog_presentation::apply(&record.window, p);
                *record.theme.borrow_mut() = revision;
            }
            continue;
        }
        let key = {
            let windows = ctx.windows.borrow();
            if windows.closing {
                continue;
            }
            windows.wanted[index]
                .clone()
                .filter(|key| key.current(p) && windows.failed[index].as_ref() != Some(key))
        };
        let Some(key) = key else {
            continue;
        };
        if !ctx.owner.is_ready() {
            continue;
        }
        match open(ctx, main, p, key.clone()) {
            Ok(record) => ctx.windows.borrow_mut().records[index] = Some(Rc::new(record)),
            Err(error) => {
                ctx.windows.borrow_mut().failed[index] = Some(key);
                // Keep the backend prompt pending; Jobs Resolve is an explicit safe retry.
                ctx.failures.record(
                    index as u8,
                    format!(
                        "Unable to open decision: {error}. Open Jobs and choose Resolve to retry."
                    ),
                );
            }
        }
    }
}
fn open(
    ctx: &Rc<Context>,
    main: &MainWindow,
    p: &Presenter,
    key: Key,
) -> Result<Record, PlatformError> {
    let session = ctx.windows.borrow_mut().sessions.allocate()?;
    let (window, host) = DialogHost::create(
        session,
        ctx.owner.clone(),
        DialogModality::Modeless,
        DecisionWindow::new,
    )?;
    crate::tooltip::install_decision(&window)?;
    dialog_presentation::apply(&window, p);
    window.set_token(key.token.as_str().into());
    if key.metadata {
        window.set_title_text("Name conflict".into());
        window.set_body(main.get_metadata_conflict_body());
        window.set_native_icon(main.get_action_icons().properties);
    } else {
        window.set_title_text(
            if main.get_copy_is_failure() {
                "Operation error"
            } else {
                "Name conflict"
            }
            .into(),
        );
        window.set_body(main.get_copy_body());
        window.set_initial_name(main.get_copy_name());
        window.set_native_icon(main.get_action_icons().copy);
        window.set_allow_alternate_name(
            !main.get_copy_is_failure() && !key.token.starts_with("rename:"),
        );
        window.set_can_replace(main.get_copy_can_replace());
        window.set_can_merge(main.get_copy_can_merge());
        window.set_failure_mode(main.get_copy_is_failure());
        window.set_can_retry(main.get_copy_can_retry());
        window.set_can_skip(main.get_copy_can_skip());
        window.set_source_details(main.get_copy_source_details());
        window.set_target_details(main.get_copy_target_details());
    }
    let weak = Rc::downgrade(ctx);
    window.on_decide(move |choice, name| {
        if let Some(ctx) = weak.upgrade() {
            respond(&ctx, session, Some((choice, name)));
        }
    });
    let weak = Rc::downgrade(ctx);
    window.on_later(move || {
        if let Some(ctx) = weak.upgrade() {
            respond(&ctx, session, None);
        }
    });
    let weak = Rc::downgrade(ctx);
    window.on_request_dismiss(move || {
        if let Some(ctx) = weak.upgrade() {
            respond(&ctx, session, Some((3, SharedString::default())));
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
    Ok(Record {
        key,
        session,
        window,
        host,
        theme: RefCell::new(theme(p)),
    })
}
fn respond(ctx: &Rc<Context>, session: DialogSession, action: Option<(i32, SharedString)>) {
    let record = {
        let windows = ctx.windows.borrow();
        if windows.closing {
            return;
        }
        windows
            .records
            .iter()
            .flatten()
            .find(|record| record.session == session && record.host.is_current(session))
            .cloned()
    };
    let (Some(record), Some(main), Some(state)) = (record, ctx.main.upgrade(), ctx.state.upgrade())
    else {
        return;
    };
    let Ok(mut p) = state.try_borrow_mut() else {
        record
            .window
            .set_warning("The view is updating. Try again.".into());
        return;
    };
    if !record.key.current(&p) {
        return;
    }
    let generation = p.desktop.notice_generation;
    if let Some((choice, name)) = &action {
        if record.key.metadata {
            p.decide_metadata_conflict(&record.key.token, *choice, ctx.send.as_ref());
        } else {
            p.decide_copy(
                &record.key.token,
                *choice,
                name.to_string(),
                ctx.send.as_ref(),
            );
        }
    } else if !record.key.metadata {
        p.defer_copy_decision(&record.key.token);
    }
    let completed = action.is_none() || !record.key.current(&p);
    if completed {
        let mut windows = ctx.windows.borrow_mut();
        windows.wanted[record.key.index()] = None;
        windows.records[record.key.index()] = None;
    } else {
        let warning = if p.desktop.notice_generation != generation {
            p.desktop.notice.take().map(|(_, body)| body)
        } else {
            None
        }
        .unwrap_or_else(|| {
            "Request queue is busy. The decision is still waiting; try again.".into()
        });
        record.window.set_warning(warning.into());
    }
    drop(p);
    if completed {
        close(Some(record), true);
    }
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
