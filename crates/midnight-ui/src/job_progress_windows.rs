//! One modeless window per exact job identity; presentation never owns backend work.
use super::{ThemeRevision, theme};
use crate::{
    JobProgressWindow, MainWindow, OperationRow, Presenter,
    dialog_host::{DialogHost, DialogModality, DialogOwner, DialogSession, DialogSessions},
    dialog_presentation::{self, DialogThemeTarget},
    presentation_eq::SamePresentation,
    tool_window_failure::Failures,
};
use midnight_core::{Command, CopyCancelConfirmationAction, OperationId};
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
#[derive(Clone, Copy)]
enum CopyConfirmation {
    WaitingForRegistration(u64),
    WaitingForCheckpoint(u64),
    Ready(u64),
    Submitted,
}

struct Record {
    window: JobProgressWindow,
    host: DialogHost,
    session: DialogSession,
    theme: ThemeRevision,
    automatic: bool,
    /// The Copy, Move, Delete or other file job this window shows, as the
    /// presenter keys it.
    job: Option<crate::job_row_cache::Key>,
    copy_id: Option<OperationId>,
    copy_observed: bool,
    copy_confirmation: Option<CopyConfirmation>,
}
impl Record {
    fn set_copy_confirmation(&mut self, confirmation: Option<CopyConfirmation>) {
        self.copy_confirmation = confirmation;
        let token = match confirmation {
            Some(CopyConfirmation::Ready(token)) => token.to_string().into(),
            _ => SharedString::default(),
        };
        self.window.set_copy_confirmation_token(token);
        self.window.set_awaiting_copy_confirmation(matches!(
            confirmation,
            Some(
                CopyConfirmation::WaitingForRegistration(_)
                    | CopyConfirmation::WaitingForCheckpoint(_)
            )
        ));
        self.window
            .set_copy_cancel_submitted(matches!(confirmation, Some(CopyConfirmation::Submitted)));
        self.window
            .set_cancel_confirmation(matches!(confirmation, Some(CopyConfirmation::Ready(_))));
    }
}
pub(super) struct Windows {
    main: slint::Weak<MainWindow>,
    state: std::rc::Weak<RefCell<Presenter>>,
    owner: DialogOwner,
    sessions: RefCell<DialogSessions>,
    next_confirmation_token: Cell<u64>,
    send: Rc<dyn Fn(Command) -> bool>,
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
            next_confirmation_token: Cell::default(),
            send: send.clone(),
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
            let mut begin = None;
            let exists = {
                let mut records = self.records.borrow_mut();
                if let Some(record) = records.get_mut(&row.identity) {
                    if record.theme != theme(p) {
                        dialog_presentation::apply(&record.window, p);
                        record.theme = theme(p);
                    }
                    let previous = record.window.get_job();
                    if previous.cancellable != row.cancellable
                        || (previous.pause_requested != row.pause_requested
                            && record.copy_confirmation.is_none())
                    {
                        record.window.set_close_error(SharedString::default());
                    }
                    // Plain equality reports a change for every row without a
                    // native icon, re-laying out and repainting the window.
                    if !previous.same_presentation(&row) {
                        record.window.set_job(row.clone());
                    }
                    if !row.active || !row.cancellable {
                        record.set_copy_confirmation(None);
                    } else if let Some(copy_id) = record.copy_id {
                        record.copy_observed = p.copy_control_available(copy_id);
                        match record.copy_confirmation {
                            Some(CopyConfirmation::WaitingForRegistration(token))
                                if record.copy_observed =>
                            {
                                begin = Some((record.session, copy_id, token));
                            }
                            Some(CopyConfirmation::WaitingForCheckpoint(token))
                                if p.copies.progress.iter().any(|progress| {
                                    progress.id == copy_id
                                        && progress.cancel_confirmation_ready == Some(token)
                                }) =>
                            {
                                record.set_copy_confirmation(Some(CopyConfirmation::Ready(token)));
                            }
                            _ => {}
                        }
                    }
                    close = record.automatic
                        && row.completed
                        && !row.failed
                        && !row.attention
                        && row.transfer_outcome.is_empty();
                    true
                } else {
                    false
                }
            };
            if let Some((session, copy_id, token)) = begin {
                self.send_copy_confirmation(
                    &row.identity,
                    session,
                    copy_id,
                    token,
                    CopyCancelConfirmationAction::Begin,
                );
            }
            if close {
                self.dismiss(&row.identity, None);
            }
            let unseen = !exists && self.seen.borrow_mut().insert(row.identity.clone());
            if unseen
                && matches!(row.kind, 0 | 1)
                && (row.active || row.failed || !row.transfer_outcome.is_empty())
            {
                self.open(p, row, true);
            }
        }
        self.publish_open_jobs(p);
    }
    /// Tells the presenter which jobs have an open window: such a window shows how
    /// the job ended, so the main window does not repeat that as a notice.
    fn publish_open_jobs(&self, p: &Presenter) {
        let jobs = self
            .records
            .borrow()
            .values()
            .filter_map(|record| record.job)
            .collect();
        p.operations.progress_windows.set(jobs);
    }
    /// [`Self::publish_open_jobs`] after a window closed outside a refresh. Only an
    /// event being applied holds the presenter; the next refresh then publishes.
    fn publish_open_jobs_now(&self) {
        if let Some(state) = self.state.upgrade()
            && let Ok(p) = state.try_borrow()
        {
            self.publish_open_jobs(&p);
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
                        && context.interactive(&id, session)
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
                && context.interactive(&id, session)
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
                && context.interactive(&id, session)
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
        let weak = Rc::downgrade(self);
        let id = row.identity.clone();
        window.on_confirm_cancel(move |token| {
            if let Some(context) = weak.upgrade() {
                context.confirm_cancel(&id, session, &token);
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
        let copy_id = row
            .identity
            .strip_prefix("copy-job:")
            .and_then(|id| id.parse().ok())
            .map(OperationId);
        // A Delete or creation row's identity is its job number alone; a Rename's
        // and a metadata edit's carry a prefix.
        let job = match row.kind {
            0 => copy_id.map(crate::job_row_cache::Key::Copy),
            1..=5 => ["rename-job:", "metadata-job:"]
                .into_iter()
                .find_map(|prefix| row.identity.strip_prefix(prefix))
                .unwrap_or(row.identity.as_str())
                .parse()
                .ok()
                .map(|id| crate::job_row_cache::Key::Mutation(OperationId(id))),
            _ => None,
        };
        self.records.borrow_mut().insert(
            row.identity,
            Record {
                window,
                host,
                session,
                theme: theme(p),
                automatic,
                job,
                copy_id,
                copy_observed: copy_id.is_some_and(|id| p.copy_control_available(id)),
                copy_confirmation: None,
            },
        );
        self.publish_open_jobs(p);
        Ok(())
    }
    fn request_close(&self, id: &SharedString, session: DialogSession) {
        if !self.current(id, session) {
            return;
        }
        let copy_confirmation = self
            .records
            .borrow()
            .get(id)
            .and_then(|record| record.copy_id.zip(record.copy_confirmation));
        if let Some((copy_id, confirmation)) = copy_confirmation {
            match confirmation {
                CopyConfirmation::WaitingForRegistration(_) => {
                    if let Some(record) = self.records.borrow_mut().get_mut(id) {
                        record.set_copy_confirmation(None);
                        record.window.set_close_error(SharedString::default());
                    }
                }
                CopyConfirmation::WaitingForCheckpoint(token) | CopyConfirmation::Ready(token) => {
                    self.send_copy_confirmation(
                        id,
                        session,
                        copy_id,
                        token,
                        CopyCancelConfirmationAction::Dismiss,
                    );
                }
                CopyConfirmation::Submitted => {}
            }
            return;
        }
        if let Some(record) = self.records.borrow().get(id)
            && record.window.get_cancel_confirmation()
        {
            record.window.set_cancel_confirmation(false);
            record.window.set_close_error(SharedString::default());
            return;
        }
        let Some(main) = self.main.upgrade() else {
            return;
        };
        if let Some(row) = main
            .get_operations()
            .iter()
            .find(|row| row.identity == *id && row.active && row.cancellable)
        {
            if row.kind == 0 {
                self.begin_copy_confirmation(id, session);
            } else if let Some(record) = self.records.borrow().get(id) {
                record.window.set_close_error(SharedString::default());
                record.window.set_cancel_confirmation(true);
            }
            return;
        }
        self.dismiss(id, Some(session));
    }
    fn begin_copy_confirmation(&self, id: &SharedString, session: DialogSession) {
        let copy = self.records.borrow().get(id).and_then(|record| {
            record
                .copy_id
                .map(|copy_id| (copy_id, record.copy_observed))
        });
        let Some((copy_id, observed)) = copy else {
            return;
        };
        let Some(token) = self.next_confirmation_token.get().checked_add(1) else {
            if let Some(record) = self.records.borrow().get(id) {
                record
                    .window
                    .set_close_error("No cancellation confirmation is available.".into());
            }
            return;
        };
        self.next_confirmation_token.set(token);
        if observed {
            self.send_copy_confirmation(
                id,
                session,
                copy_id,
                token,
                CopyCancelConfirmationAction::Begin,
            );
        } else if let Some(record) = self.records.borrow_mut().get_mut(id) {
            // An optimistic queued receipt can appear before the worker registers
            // this ID. Defer Begin until the first actual live snapshot.
            record.set_copy_confirmation(Some(CopyConfirmation::WaitingForRegistration(token)));
            record.window.set_close_error(SharedString::default());
        }
    }
    fn send_copy_confirmation(
        &self,
        id: &SharedString,
        session: DialogSession,
        copy_id: OperationId,
        token: u64,
        action: CopyCancelConfirmationAction,
    ) {
        if !self.current(id, session) {
            return;
        }
        use CopyCancelConfirmationAction::{Begin, Confirm, Dismiss};
        // This value-only control needs no ownership of captured paths or sources.
        let accepted = (self.send)(Command::CopyCancelConfirmation {
            id: copy_id,
            token,
            action,
        });
        let mut records = self.records.borrow_mut();
        let Some(record) = records
            .get_mut(id)
            .filter(|record| record.session == session)
        else {
            return;
        };
        if accepted {
            record.set_copy_confirmation(match action {
                Begin => Some(CopyConfirmation::WaitingForCheckpoint(token)),
                Dismiss => None,
                Confirm => Some(CopyConfirmation::Submitted),
            });
            record.window.set_close_error(SharedString::default());
        } else {
            let message = match action {
                Begin => {
                    record.set_copy_confirmation(None);
                    "Pause could not be requested. Try Cancel operation again."
                }
                Dismiss => "The pause could not be released. Try Continue again.",
                Confirm => "Cancellation could not be queued. Try Cancel operation again.",
            };
            record.window.set_close_error(message.into());
        }
    }
    fn confirm_cancel(
        &self,
        id: &SharedString,
        session: DialogSession,
        shown_token: &SharedString,
    ) {
        if !self.current(id, session)
            || !self
                .records
                .borrow()
                .get(id)
                .is_some_and(|record| record.window.get_cancel_confirmation())
        {
            return;
        }
        let Some(main) = self.main.upgrade() else {
            return;
        };
        // Completion or a cancellation from another surface can win while the
        // question is open. Only the current live row grants cancellation.
        let Some(row) = main
            .get_operations()
            .iter()
            .find(|row| row.identity == *id && row.active && row.cancellable)
        else {
            if let Some(record) = self.records.borrow_mut().get_mut(id) {
                record.set_copy_confirmation(None);
            }
            return;
        };
        if row.kind == 0 {
            let ready = self.records.borrow().get(id).and_then(|record| {
                let Some(CopyConfirmation::Ready(token)) = record.copy_confirmation else {
                    return None;
                };
                if shown_token.as_str().parse().ok() != Some(token) {
                    return None;
                }
                record.copy_id.map(|copy_id| (copy_id, token))
            });
            if let Some((copy_id, token)) = ready {
                self.send_copy_confirmation(
                    id,
                    session,
                    copy_id,
                    token,
                    CopyCancelConfirmationAction::Confirm,
                );
            }
            return;
        }
        main.invoke_cancel_job(id.clone());
        // Admission is reflected synchronously by the exact row. A full backend
        // queue must leave the question and its retry action visible.
        if main
            .get_operations()
            .iter()
            .any(|row| row.identity == *id && row.active && row.cancellable)
        {
            if let Some(record) = self.records.borrow().get(id) {
                record.window.set_close_error(
                    "Cancellation could not be queued. Try Cancel operation again.".into(),
                );
            }
            return;
        }
        // As a Copy's, the window stays to show how the job ends, so no notice
        // has to report it.
        if let Some(record) = self.records.borrow().get(id) {
            record.window.set_cancel_confirmation(false);
            record.window.set_close_error(SharedString::default());
        }
    }
    fn interactive(&self, id: &SharedString, session: DialogSession) -> bool {
        self.current(id, session)
            && self.records.borrow().get(id).is_some_and(|record| {
                record.copy_confirmation.is_none() && !record.window.get_cancel_confirmation()
            })
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
            self.publish_open_jobs_now();
        }
    }
    pub(super) fn close_all(&self) {
        self.closing.set(true);
        let records = std::mem::take(&mut *self.records.borrow_mut());
        for record in records.into_values() {
            record.host.close(record.window.window());
        }
        self.publish_open_jobs_now();
    }
}
