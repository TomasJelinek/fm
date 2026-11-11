//! One modeless window per exact job identity; presentation never owns backend work.
use super::{ThemeRevision, theme};
use crate::{
    JobProgressWindow, MainWindow, OperationRow, Presenter,
    dialog_host::{DialogHost, DialogModality, DialogOwner, DialogSession, DialogSessions},
    dialog_presentation::{self, DialogThemeTarget},
    tool_window_failure::Failures,
};
use midnight_core::Command;
use slint::{ComponentHandle, Model, SharedString};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
};

impl DialogThemeTarget for JobProgressWindow {
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
    window: JobProgressWindow,
    host: DialogHost,
    session: DialogSession,
    theme: ThemeRevision,
    automatic: bool,
}
pub(super) struct Windows {
    main: slint::Weak<MainWindow>,
    state: std::rc::Weak<RefCell<Presenter>>,
    owner: DialogOwner,
    sessions: RefCell<DialogSessions>,
    records: RefCell<HashMap<SharedString, Record>>,
    seen: RefCell<HashSet<SharedString>>,
    closing: Rc<Cell<bool>>,
    failures: Failures<u8>,
}
impl Windows {
    pub(super) fn install(
        main: &MainWindow,
        state: &Rc<RefCell<Presenter>>,
        send: &Rc<dyn Fn(Command) -> bool>,
    ) -> Rc<Self> {
        let closing = Rc::new(Cell::new(false));
        let this = Rc::new(Self {
            main: main.as_weak(),
            state: Rc::downgrade(state),
            owner: DialogOwner::new(main),
            sessions: RefCell::default(),
            records: RefCell::default(),
            seen: RefCell::default(),
            failures: Failures::new(
                main,
                state,
                send,
                &closing,
                "Operation progress",
                |_, _, _| true,
            ),
            closing,
        });
        let context = this.clone();
        main.on_open_job_progress(move |identity| {
            let (Some(main), Some(state)) = (context.main.upgrade(), context.state.upgrade())
            else {
                return;
            };
            if let Some(row) = main
                .get_operations()
                .iter()
                .find(|row| row.identity == identity)
            {
                context.open(&state.borrow(), row, false);
            }
        });
        this
    }
    pub(super) fn synchronize(self: &Rc<Self>, main: &MainWindow, p: &Presenter) {
        if self.closing.get() {
            return;
        }
        if p.operations.closing {
            self.close_all();
            return;
        }
        if !self.owner.is_ready() {
            return;
        }
        let rows: Vec<_> = main.get_operations().iter().collect();
        let live: HashSet<_> = rows.iter().map(|row| row.identity.clone()).collect();
        let stale: Vec<_> = self
            .records
            .borrow()
            .keys()
            .filter(|id| !live.contains(*id))
            .cloned()
            .collect();
        for id in stale {
            self.dismiss(&id, None);
        }
        self.seen.borrow_mut().retain(|id| live.contains(id));
        for row in rows {
            let mut close = false;
            let exists = {
                let mut records = self.records.borrow_mut();
                if let Some(record) = records.get_mut(&row.identity) {
                    if record.theme != theme(p) {
                        dialog_presentation::apply(&record.window, p);
                        record.theme = theme(p);
                    }
                    let previous = record.window.get_job();
                    if previous.pause_requested != row.pause_requested
                        || previous.cancellable != row.cancellable
                    {
                        record.window.set_close_error(SharedString::default());
                    }
                    record.window.set_job(row.clone());
                    close = record.automatic && row.completed && !row.failed && !row.attention;
                    true
                } else {
                    false
                }
            };
            if close {
                self.dismiss(&row.identity, None);
            }
            let unseen = !exists && self.seen.borrow_mut().insert(row.identity.clone());
            if unseen && matches!(row.kind, 0 | 1) && (row.active || row.failed) {
                self.open(p, row, true);
            }
        }
    }
    fn open(self: &Rc<Self>, p: &Presenter, row: OperationRow, automatic: bool) {
        if self.closing.get() || p.operations.closing || !self.owner.is_ready() {
            return;
        }
        if let Some(record) = self.records.borrow_mut().get_mut(&row.identity) {
            if !automatic {
                record.automatic = false;
                record.host.activate(record.window.window());
            }
            return;
        }
        self.seen.borrow_mut().insert(row.identity.clone());
        if let Err(error) = self.create(p, row, automatic) {
            self.failures.record(0, format!("Unable to open operation progress: {error}. The operation remains in Jobs; choose Open to retry."));
        }
    }
    fn create(
        self: &Rc<Self>,
        p: &Presenter,
        row: OperationRow,
        automatic: bool,
    ) -> Result<(), slint::PlatformError> {
        let session = self.sessions.borrow_mut().allocate()?;
        let (window, host) = DialogHost::create(
            session,
            self.owner.clone(),
            DialogModality::Modeless,
            JobProgressWindow::new,
        )?;
        crate::tooltip::install_job_progress(&window)?;
        dialog_presentation::apply(&window, p);
        window.set_job(row.clone());
        macro_rules! action {
            ($callback:ident, $invoke:ident) => {{
                let weak = Rc::downgrade(self);
                let id = row.identity.clone();
                window.$callback(move || {
                    if let Some(context) = weak.upgrade()
                        && context.current(&id, session)
                        && let Some(main) = context.main.upgrade()
                    {
                        main.$invoke(id.clone());
                    }
                });
            }};
        }
        action!(on_resolve, invoke_resolve_copy);
        action!(on_copy_details, invoke_copy_job_details);
        let weak = Rc::downgrade(self);
        let id = row.identity.clone();
        window.on_set_paused(move |paused| {
            if let Some(context) = weak.upgrade()
                && context.current(&id, session)
                && let Some(main) = context.main.upgrade()
            {
                main.invoke_pause_job(id.clone(), paused);
                if let Some(row) = main.get_operations().iter().find(|row| row.identity == id)
                    && row.active
                    && row.pause_requested != paused
                    && let Some(record) = context.records.borrow().get(&id)
                {
                    record
                        .window
                        .set_close_error("Pause request could not be queued. Try again.".into());
                }
            }
        });
        let weak = Rc::downgrade(self);
        let id = row.identity.clone();
        window.on_show_jobs(move || {
            if let Some(context) = weak.upgrade()
                && context.current(&id, session)
                && let Some(main) = context.main.upgrade()
            {
                main.invoke_show_operations();
            }
        });
        let weak = Rc::downgrade(self);
        let id = row.identity.clone();
        window.on_request_dismiss(move || {
            if let Some(context) = weak.upgrade() {
                context.request_close(&id, session);
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
        if !automatic {
            host.activate(window.window());
        }
        self.records.borrow_mut().insert(
            row.identity,
            Record {
                window,
                host,
                session,
                theme: theme(p),
                automatic,
            },
        );
        Ok(())
    }
    fn request_close(&self, id: &SharedString, session: DialogSession) {
        if !self.current(id, session) {
            return;
        }
        let Some(main) = self.main.upgrade() else {
            return;
        };
        let row = main.get_operations().iter().find(|row| row.identity == *id);
        if row
            .as_ref()
            .is_some_and(|row| row.active && row.cancellable)
        {
            main.invoke_cancel_job(id.clone());
            // Admission is reflected synchronously by the exact row. A full
            // backend queue must leave an actionable window, never silently hide it.
            if main
                .get_operations()
                .iter()
                .any(|row| row.identity == *id && row.active && row.cancellable)
            {
                if let Some(record) = self.records.borrow().get(id) {
                    record.window.set_close_error(
                        "Cancellation could not be queued. Try Cancel again.".into(),
                    );
                }
                return;
            }
        }
        self.dismiss(id, Some(session));
    }
    fn current(&self, id: &SharedString, session: DialogSession) -> bool {
        !self.closing.get()
            && self
                .records
                .borrow()
                .get(id)
                .is_some_and(|record| record.session == session && record.host.is_current(session))
    }
    fn dismiss(&self, id: &SharedString, session: Option<DialogSession>) {
        if session.is_some_and(|session| !self.current(id, session)) {
            return;
        }
        let record = self.records.borrow_mut().remove(id);
        if let Some(record) = record {
            record.host.close(record.window.window());
        }
    }
    pub(super) fn close_all(&self) {
        self.closing.set(true);
        let records = std::mem::take(&mut *self.records.borrow_mut());
        for record in records.into_values() {
            record.host.close(record.window.window());
        }
    }
}
