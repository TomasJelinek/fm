use super::mailbox::Input;
use super::{Generation, Mailbox, command_navigation, diagnostic_rejection, rejection};
use crate::{
    Outbox, Snapshots,
    providers::{NativeProvider, ProviderError, ProviderRegistry},
    tree_actor::{NAMESPACE_POLL, Session, Wait, WorkerBudget},
    tree_projection::ProjectionOptions,
    tree_roots::ReadyRoot,
};
use midnight_core::{
    Event, LocaleService, NavigationId, SelectionRevision, TreeCommand, TreeEvent,
    TreeRootSelection, TreeRootSnapshot,
};
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[path = "worker_output.rs"]
mod output;
#[cfg(test)]
#[path = "worker_refresh_tests.rs"]
mod refresh_tests;

struct Pending {
    root: TreeRootSnapshot,
    revision: SelectionRevision,
    selected: TreeRootSelection,
    ready: Option<Arc<ReadyRoot>>,
}

/// Install watch authority from the session's exactly acknowledged current
/// projection. Returns a watch failure the UI has not seen yet.
fn update_watch_scope(watch: &crate::watch::Handle, session: &mut Session) -> Option<TreeEvent> {
    match session.watch_scope() {
        Ok(Some(scope)) => {
            watch.set_tree_scope(scope);
            None
        }
        Ok(None) => None,
        Err(error) => {
            let stamp = session.projection().stamp;
            let error = session.watch_failure_diagnostic(&error);
            watch
                .scope_failed(stamp, session.watch_eligibility(), error.clone())
                .then_some(TreeEvent::WatchFailed { stamp, error })
        }
    }
}
pub(super) struct Services {
    pub rename: crate::tree_rename::Retirement,
    pub native: NativeProvider,
    pub locale: Option<Arc<dyn LocaleService>>,
    pub reports: crate::ReportArena,
}
pub(super) struct Worker {
    rename: crate::tree_rename::Retirement,
    reports: crate::ReportArena,
    inbox: Arc<Mailbox>,
    snapshots: Snapshots,
    native: NativeProvider,
    locale: Option<Arc<dyn LocaleService>>,
    listing: Arc<Semaphore>,
    budget: WorkerBudget,
    registry: Arc<ProviderRegistry>,
    outbox: Outbox,
    generation: Option<Generation>,
    completed: Option<NavigationId>,
    pending: Option<Pending>,
    session: Option<Session>,
    // A published view remains admissible until the UI acknowledges its exact Arc.
    // One newer projection can coalesce on this model worker while delivery waits.
    awaiting_commit: Option<Arc<midnight_core::TreeProjection>>,
    pending_publication: Option<midnight_core::TreePublication>,
    inflight: bool,
    maintenance_inflight: bool,
    sizes: crate::tree_sizes::State,
    copy_capacity: Arc<Semaphore>,
    pub(super) watch: Option<crate::watch::Handle>,
    refresh_ticket: Option<crate::watch::RefreshTicket>,
    /// Test observer, notified each time `run` waits to retry namespace admission.
    #[cfg(test)]
    namespace_retries: Option<Arc<tokio::sync::Notify>>,
}
impl Worker {
    pub fn new(
        inbox: Arc<Mailbox>,
        snapshots: Snapshots,
        services: Services,
        budget: WorkerBudget,
        outbox: Outbox,
        copy_capacity: Arc<Semaphore>,
    ) -> Self {
        let Services {
            native,
            locale,
            reports,
            rename,
        } = services;
        let listing = budget.listing();
        Self {
            rename,
            watch: None,
            refresh_ticket: None,
            copy_capacity,
            sizes: crate::tree_sizes::State::new(
                native.filesystem(),
                inbox.wake.clone(),
                reports.clone(),
            ),
            reports,
            inbox,
            snapshots,
            native,
            locale,
            listing,
            budget,
            outbox,
            registry: Arc::default(),
            generation: None,
            completed: None,
            pending: None,
            session: None,
            awaiting_commit: None,
            pending_publication: None,
            inflight: false,
            maintenance_inflight: false,
            #[cfg(test)]
            namespace_retries: None,
        }
    }
    pub fn run(mut self) {
        let runtime = tokio::runtime::Handle::current();
        loop {
            self.pump_sizes();
            let inbox = self.inbox.clone();
            let timed = self.sizes.busy();
            let notified = async {
                if timed {
                    let _ = tokio::time::timeout(
                        std::time::Duration::from_millis(250),
                        inbox.wake.notified(),
                    )
                    .await;
                } else {
                    inbox.wake.notified().await;
                }
            };
            if let Some(input) = inbox.pop() {
                if matches!(input, Input::Closed) {
                    break;
                }
                self.handle(input);
                self.admit();
                self.pump();
                self.pump_sizes();
                continue;
            }
            if self
                .pending
                .as_ref()
                .is_some_and(|pending| pending.ready.is_some())
            {
                let listing = self.listing.clone();
                let permit = runtime.block_on(async {
                    tokio::select! {
                        _ = notified => None,
                        permit = listing.acquire_owned() => permit.ok(),
                    }
                });
                if let Some(permit) = permit {
                    self.construct(permit);
                    self.pump();
                }
            } else if !self.inflight
                && self.pending_publication.is_none()
                && let Some(session) = self.session.as_ref()
                && session.queued() > 0
            {
                match session.waiting_on() {
                    // Wait on the pane semaphore, never poll for native workers to return.
                    Wait::Permit(semaphore) => runtime.block_on(async {
                        tokio::select! {
                            _ = notified => {},
                            permit = semaphore.acquire_owned() => drop(permit),
                        }
                    }),
                    // Queueing on the fair transfer-namespace semaphore would reserve
                    // released transfer shares and block new transfers. Retry admission.
                    Wait::Namespace => {
                        #[cfg(test)]
                        if let Some(retries) = &self.namespace_retries {
                            retries.notify_one();
                        }
                        runtime.block_on(async {
                            let _ = tokio::time::timeout(NAMESPACE_POLL, notified).await;
                        });
                    }
                }
                self.pump();
            } else {
                runtime.block_on(notified);
            }
        }
        self.inbox.drain();
        // Session, pending source, queued commands and completion resources are
        // all destroyed on this worker, including disconnected-input shutdown.
    }
    fn active(&self) -> bool {
        self.generation
            .as_ref()
            .is_some_and(|generation| !generation.cancel.is_cancelled())
    }
    /// Install and release completion resources before producing any output.
    fn complete(&mut self, completion: crate::tree_actor::BranchCompletion) -> Option<TreeEvent> {
        self.inflight = false;
        self.maintenance_inflight = false;
        if !self.active() {
            return None;
        }
        let generation = self.generation.as_ref().expect("active generation");
        let result = self.session.as_mut().map(|session| {
            session
                .apply(completion, &generation.cancel)
                .and_then(|changed| {
                    if changed {
                        session.publication().map(Some)
                    } else {
                        Ok(None)
                    }
                })
        });
        match result {
            Some(Ok(Some(publication))) => Some(TreeEvent::Published(publication)),
            Some(Err(error)) => self.end_session(error),
            _ => None,
        }
    }
    /// The pane's current Show hidden files choice (`Command::ShowHiddenEntries`).
    fn hide_hidden(&self, pane: midnight_core::PaneId) -> bool {
        self.snapshots.lock().unwrap_or_else(|e| e.into_inner())[pane.index()].hide_hidden
    }
    fn handle(&mut self, input: Input) {
        // Every build this input causes uses the current hidden-files choice.
        if let Some(pane) = self
            .session
            .as_ref()
            .map(|session| session.projection().stamp.pane)
        {
            let hide = self.hide_hidden(pane);
            if let Some(session) = &mut self.session {
                session.set_hide_hidden(hide);
            }
        }
        match input {
            Input::Closed => unreachable!(),
            Input::Reset(generation) => {
                self.refresh_ticket = None;
                self.sizes.reset();
                self.awaiting_commit = None;
                self.pending_publication = None;
                self.session = None;
                self.pending = None;
                self.completed = None;
                self.generation = Some(generation);
                // An old branch still owns the pane budget until its completion.
            }
            Input::Ready(navigation) => {
                if self.completed.is_none_or(|old| old < navigation) {
                    self.completed = Some(navigation);
                }
            }
            Input::Completion(completion) => {
                if let Some(event) = self.complete(completion) {
                    self.emit(event);
                }
            }
            Input::Command(command) => {
                if !self.active()
                    || self.generation.as_ref().is_none_or(|generation| {
                        generation.navigation != command_navigation(&command)
                    })
                {
                    return;
                }
                self.command(command);
            }
        }
    }
    fn command(&mut self, command: TreeCommand) {
        if self.session.is_none()
            && matches!(
                &command,
                TreeCommand::Commit(_) | TreeCommand::Viewport { .. }
            )
        {
            // A failed session already reported its terminal Unavailable event.
            // Observing its last publication has no effect; a Rejected reply would
            // make the UI commit again and loop until navigation.
            return;
        }
        // A completion can overtake a command already admitted by the mailbox.
        // Retire obsolete observations here as well, on the model worker.
        let emitted_commit = matches!(&command, TreeCommand::Commit(view)
            if self.awaiting_commit.as_ref().is_some_and(|emitted| Arc::ptr_eq(emitted, view)));
        if !emitted_commit
            && self.session.as_ref().is_some_and(|session| {
                super::superseded_observation(&command, session.projection().stamp)
            })
        {
            return;
        }
        let observation = matches!(
            &command,
            TreeCommand::Commit(_)
                | TreeCommand::Viewport { .. }
                | TreeCommand::Sizes(midnight_core::TreeSizeCommand::Acknowledge { .. })
        );
        if !observation && let Some(session) = &mut self.session {
            session.rollback_unpublished();
        }
        if let TreeCommand::Sizes(command) = command {
            self.size_command(command);
            return;
        }
        if let TreeCommand::Enable {
            root,
            selection_revision,
            selected,
        } = command
        {
            if let Some(session) = self.session.as_mut()
                && session.projection().root.same_snapshot(&root)
            {
                let result = session
                    .seed_selection(selection_revision, &selected)
                    .and_then(|()| session.publication());
                match result {
                    Ok(publication) => self.emit(TreeEvent::Published(publication)),
                    Err(error) => self.emit(TreeEvent::Unavailable {
                        root,
                        error: crate::tree_diagnostics::diagnostic(&self.reports, &error),
                    }),
                }
            } else {
                self.pending = Some(Pending {
                    root,
                    revision: selection_revision,
                    selected,
                    ready: None,
                });
            }
            return;
        }
        let Some(session) = self.session.as_mut() else {
            self.emit(rejection(
                command,
                "Tree session is not ready.",
                self.pending.is_some(),
            ));
            return;
        };
        let cancel = &self.generation.as_ref().expect("active generation").cancel;
        let mut size_events = Vec::new();
        let result: Result<Option<TreeEvent>, ProviderError> = match &command {
            TreeCommand::RefreshLoaded(request) => {
                session.refresh_loaded(request, cancel).map(|()| {
                    self.refresh_ticket = self
                        .watch
                        .as_ref()
                        .and_then(|watch| watch.begin_refresh(request.view.stamp, request.request));
                    size_events = self.sizes.invalidate();
                    session.size_sort(Arc::default());
                    None
                })
            }
            TreeCommand::Sizes(_) => unreachable!(),
            TreeCommand::CaptureDropDirectory {
                id,
                view,
                request,
                row,
            } => Ok(Some(TreeEvent::DropDirectoryCaptured {
                id: *id,
                stamp: view.stamp,
                request: *request,
                result: session
                    .capture_drop_directory(view, *row, *request, &self.copy_capacity, cancel)
                    .map_err(|error| crate::tree_diagnostics::diagnostic(&self.reports, &error)),
            })),
            TreeCommand::Metadata(request) => {
                let result =
                    session.capture_metadata(request, &self.copy_capacity, &self.rename, cancel);
                let retryable = matches!(&result, Err(ProviderError::ResourceLimited));
                Ok(Some(TreeEvent::MetadataCaptured {
                    stamp: request.view.stamp,
                    request: request.request,
                    retryable,
                    result: result.map_err(|error| {
                        crate::tree_diagnostics::diagnostic(&self.reports, &error)
                    }),
                }))
            }
            TreeCommand::Rename(request) => Ok(Some(TreeEvent::RenameCaptured {
                stamp: request.view.stamp,
                request: request.request,
                result: session
                    .capture_rename(request, &self.copy_capacity, &self.rename, cancel)
                    .map_err(|error| crate::tree_diagnostics::diagnostic(&self.reports, &error)),
            })),
            TreeCommand::Edit(request) => Ok(Some(TreeEvent::EditorCaptured {
                stamp: request.view.stamp,
                request: request.request,
                result: session
                    .capture_editor(request, &self.copy_capacity, cancel)
                    .map_err(|error| crate::tree_diagnostics::diagnostic(&self.reports, &error)),
            })),
            TreeCommand::UserCommand(request) => {
                let result = session.capture_user_command(
                    request,
                    &self.copy_capacity,
                    &self.rename,
                    cancel,
                );
                let retryable = matches!(&result, Err(ProviderError::ResourceLimited));
                Ok(Some(TreeEvent::UserCommandCaptured {
                    stamp: request.view.stamp,
                    request: request.request,
                    retryable,
                    result: result.map_err(|error| {
                        crate::tree_diagnostics::diagnostic(&self.reports, &error)
                    }),
                }))
            }
            TreeCommand::Checksums(request) => Ok(Some(TreeEvent::ChecksumsCaptured {
                stamp: request.view.stamp,
                request: request.request,
                result: session
                    .capture_checksums(request, &self.copy_capacity, cancel)
                    .map_err(|error| crate::tree_diagnostics::diagnostic(&self.reports, &error)),
            })),
            TreeCommand::Clipboard(request) => Ok(Some(TreeEvent::Clipboard {
                stamp: request.view.stamp,
                request: request.request,
                result: session
                    .capture_clipboard(request, &self.copy_capacity, cancel)
                    .map_err(|error| crate::tree_diagnostics::diagnostic(&self.reports, &error)),
            })),
            TreeCommand::Delete(request) => Ok(Some(TreeEvent::DeleteCaptured {
                stamp: request.view.stamp,
                request: request.request,
                result: session
                    .capture_delete(request, &self.copy_capacity, cancel)
                    .map_err(|error| crate::tree_diagnostics::diagnostic(&self.reports, &error)),
            })),
            TreeCommand::Transfer(request) => {
                let result = session.capture_transfer(request, &self.copy_capacity, cancel);
                // A newer projection replaced the admitted view: the presenter
                // captures again once that publication is shown. Other refusals
                // are reported, because ResourceLimited stands for a busy permit
                // pool and for a selection over the capture budget alike.
                let retryable =
                    result.is_err() && !Arc::ptr_eq(session.projection(), &request.view);
                Ok(Some(TreeEvent::Transfer {
                    stamp: request.view.stamp,
                    request: request.request,
                    retryable,
                    result: result.map_err(|error| {
                        crate::tree_diagnostics::diagnostic(&self.reports, &error)
                    }),
                }))
            }
            TreeCommand::Commit(view) => {
                if session.commit(view) || (emitted_commit && session.acknowledge_emitted(view)) {
                    if emitted_commit {
                        self.awaiting_commit = None;
                    }
                    // Watches and selection captures still require the latest
                    // projection, even when a visible row gesture can survive it.
                    if Arc::ptr_eq(session.projection(), view)
                        && let Some(watch) = &self.watch
                    {
                        Ok(update_watch_scope(watch, session))
                    } else {
                        Ok(None)
                    }
                } else {
                    Err(ProviderError::StaleReference)
                }
            }
            TreeCommand::Branch {
                view,
                request,
                row,
                action,
            } => {
                session
                    .branch(view, *request, *row, *action)
                    .and_then(|()| {
                        if *action == midnight_core::TreeBranchAction::Refresh {
                            // Refresh must not restart snapshot versions within a live session.
                            // Running owners keep their slot until actual provider return.
                            size_events = self.sizes.invalidate();
                            session.size_sort(Arc::default());
                        }
                        if *action == midnight_core::TreeBranchAction::Collapse {
                            session.update_projection(cancel)?;
                            session
                                .publication()
                                .map(|publication| Some(TreeEvent::Published(publication)))
                        } else {
                            Ok(None)
                        }
                    })
            }
            TreeCommand::Viewport { view, rows } => {
                session.viewport(view, rows.rows()).map(|()| None)
            }
            TreeCommand::Select(request) => session.select(request).and_then(|()| {
                let publication = session.publication()?;
                Ok(Some(TreeEvent::SelectionChanged {
                    view: publication.view,
                    revision: publication.selection_revision,
                    selection: publication.selection,
                }))
            }),
            TreeCommand::Reproject(request) => {
                let previous = session.size_sort(self.sizes.sort_sizes());
                let result = session
                    .reproject_request(request, cancel)
                    .map(|publication| {
                        Some(TreeEvent::Reprojected {
                            previous: request.view.stamp,
                            request: request.request,
                            options: request.options.clone(),
                            publication,
                        })
                    });
                if result.is_err() {
                    session.size_sort(previous);
                }
                result
            }
            TreeCommand::Navigate {
                view,
                request,
                row,
                navigation,
            } => {
                // Context validation may call the archive provider. Saturation is
                // explicit, and the permit stays with this worker until return.
                self.listing
                    .clone()
                    .try_acquire_owned()
                    .map_err(|_| ProviderError::ResourceLimited)
                    .and_then(|_permit| {
                        session
                            .navigate(view, *request, *row, *navigation, cancel)
                            .map(|target| {
                                Some(TreeEvent::Navigated {
                                    stamp: view.stamp,
                                    request: *request,
                                    row: *row,
                                    navigation: *navigation,
                                    target,
                                })
                            })
                    })
            }
            TreeCommand::Enable { .. } => unreachable!(),
        };
        for event in size_events {
            self.emit(TreeEvent::Sizes(event));
        }
        match result {
            Ok(Some(event)) => self.emit(event),
            Err(error) => {
                let retryable = matches!(error, ProviderError::StaleReference)
                    || matches!(
                        (&command, &error),
                        (
                            TreeCommand::RefreshLoaded(_),
                            ProviderError::ResourceLimited
                        )
                    );
                self.emit(diagnostic_rejection(
                    command,
                    crate::tree_diagnostics::diagnostic(&self.reports, &error),
                    retryable,
                ));
            }
            Ok(None) => {}
        }
        if self.awaiting_commit.is_none()
            && let Some(publication) = self.pending_publication.take()
        {
            self.emit(TreeEvent::Published(publication));
        }
        self.emit_size_selection();
    }
    fn admit(&mut self) {
        if !self.active() {
            return;
        }
        let Some(pending) = &mut self.pending else {
            return;
        };
        if pending.ready.is_some() {
            return;
        }
        let decision = {
            let states = self.snapshots.lock().unwrap_or_else(|e| e.into_inner());
            let state = &states[pending.root.pane.index()];
            match state.capture_tree_root(&pending.root) {
                Ok(ready) => Ok(Some(ready)),
                Err(_)
                    if self
                        .completed
                        .is_none_or(|done| done < pending.root.navigation)
                        && (state.pending == Some(pending.root.navigation)
                            || (state.archive_navigation == Some(pending.root.navigation)
                                && state.tree_root.as_ref().is_none_or(|root| {
                                    root.snapshot().navigation != pending.root.navigation
                                }))) =>
                {
                    Ok(None)
                }
                Err(error) => Err(error),
            }
        };
        match decision {
            Ok(ready) => pending.ready = ready,
            Err(error) => {
                let pending = self.pending.take().expect("pending root");
                self.emit(TreeEvent::Unavailable {
                    root: pending.root,
                    error: crate::tree_diagnostics::diagnostic(&self.reports, &error),
                });
            }
        }
    }
    fn construct(&mut self, _permit: OwnedSemaphorePermit) {
        if !self.active() {
            return;
        }
        let Some(pending) = self.pending.take() else {
            return;
        };
        let Some(ready) = pending.ready else {
            return;
        };
        let hide_hidden = self.hide_hidden(pending.root.pane);
        let generation = self.generation.as_ref().expect("active generation");
        let result = ready
            .open(&self.native, &generation.cancel)
            .and_then(|opened| {
                let mut session = Session::new(
                    pending.root.clone(),
                    opened,
                    self.registry.clone(),
                    self.budget.clone(),
                    ProjectionOptions {
                        sort: generation.sort,
                        locale: self.locale.clone(),
                        hide_hidden,
                        ..Default::default()
                    },
                    &generation.cancel,
                    self.reports.clone(),
                )?;
                session.seed_selection(pending.revision, &pending.selected)?;
                let publication = session.publication()?;
                Ok((session, publication))
            });
        match result {
            Ok((session, publication)) if self.active() => {
                self.session = Some(session);
                self.emit(TreeEvent::Published(publication));
            }
            Ok(_) => {}
            Err(error) => self.emit(TreeEvent::Unavailable {
                root: pending.root,
                error: crate::tree_diagnostics::diagnostic(&self.reports, &error),
            }),
        }
    }
    fn pump(&mut self) {
        if self.inflight
            || !self.active()
            || self.pending_publication.is_some()
            || self.inbox.has_pending_input()
        {
            return;
        }
        let Some(session) = &mut self.session else {
            return;
        };
        match session.take_work() {
            Ok(Some(job)) => {
                let maintenance = job.maintenance();
                // An explicit Expand/Refresh shows its Loading row. Probes are
                // optional read-ahead and publish only their result.
                if !maintenance && !job.recursive() && !job.probe() {
                    let generation = self.generation.as_ref().expect("active generation");
                    // The Loading marker is cosmetic: if it cannot be projected, the
                    // completion still installs and publishes the listing.
                    if let Ok(publication) = session
                        .update_projection(&generation.cancel)
                        .and_then(|()| session.publication())
                    {
                        self.emit(TreeEvent::Published(publication));
                    }
                }
                self.inflight = true;
                self.maintenance_inflight = maintenance;
                self.inbox.start_branch(job);
            }
            Err(error) => {
                if let Some(event) = self
                    .session
                    .as_mut()
                    .and_then(|session| session.abort_refresh(&error))
                {
                    self.emit_refreshed(event);
                } else {
                    self.fail_session(error);
                }
            }
            Ok(None) => {}
        }
        self.flush_notices();
        self.finish_refresh_pass();
    }
    /// Reply to a finished refresh pass, but never ahead of a publication of
    /// that pass still parked behind an unacknowledged view (`emit`): the UI
    /// ends a file job's input hold on the reply and reveals a renamed entry
    /// only in a published view. The Commit that sends the parked publication
    /// is followed by another `pump`, which replies then.
    fn finish_refresh_pass(&mut self) {
        if self.pending_publication.is_some() {
            return;
        }
        if let Some(event) = self.session.as_mut().and_then(Session::finish_refresh) {
            self.emit_refreshed(event);
        }
    }
    fn emit_refreshed(&mut self, event: TreeEvent) {
        let watch_failure = self.finish_watch_refresh(&event);
        self.emit(event);
        if let Some(failure) = watch_failure {
            self.emit(failure);
        }
    }
    fn flush_notices(&mut self) {
        if let Some(publication) = self
            .session
            .as_mut()
            .and_then(Session::take_recursive_publication)
        {
            self.emit(TreeEvent::Published(publication));
        }
        // Bounded: emitting can apply only a maintenance completion, which adds no
        // notice, and a failed session ends the loop.
        let take = |session: &mut Session| {
            session
                .take_recursive_notice()
                .or_else(|| session.take_branch_notice())
        };
        while let Some(mut event) = self.session.as_mut().and_then(take) {
            // The UI accepts row diagnostics only for the exact emitted view.
            // A newer private projection may still be coalesced behind its Commit.
            if let TreeEvent::Rejected { stamp, .. } = &mut event
                && let Some(emitted) = &self.awaiting_commit
            {
                *stamp = emitted.stamp;
            }
            self.emit(event);
        }
    }
    /// Returns a watch failure to emit after the refresh reply.
    fn finish_watch_refresh(&mut self, event: &TreeEvent) -> Option<TreeEvent> {
        let TreeEvent::Refreshed { result, .. } = event else {
            return None;
        };
        self.finish_watch_ticket(result.is_ok());
        // A pass whose listings changed nothing visible publishes nothing, so no
        // later Commit would settle its fence. The current projection is already
        // exactly acknowledged: install the refreshed scope from it now.
        if result.is_ok()
            && let (Some(watch), Some(session)) = (&self.watch, self.session.as_mut())
            && session.projection_accepted()
        {
            return update_watch_scope(watch, session);
        }
        None
    }
    fn finish_watch_ticket(&mut self, success: bool) {
        if let Some(ticket) = self.refresh_ticket.take()
            && let Some(watch) = &self.watch
        {
            let revision = self
                .session
                .as_ref()
                .map_or(midnight_core::TreeRevision(0), |session| {
                    session.projection().stamp.revision
                });
            watch.finish_refresh(ticket, success, revision);
        }
    }
    fn fail_session(&mut self, error: ProviderError) {
        if let Some(event) = self.end_session(error) {
            self.emit(event);
        }
    }
    /// Drop the session and return its terminal event. Later Commit/Viewport
    /// observations of its last publication are ignored (see `command`).
    fn end_session(&mut self, error: ProviderError) -> Option<TreeEvent> {
        self.awaiting_commit = None;
        self.pending_publication = None;
        self.finish_watch_ticket(false);
        self.retire_watch();
        self.session.take().map(|session| TreeEvent::Unavailable {
            root: session.projection().root.clone(),
            error: crate::tree_diagnostics::diagnostic(&self.reports, &error),
        })
    }
    fn retire_watch(&self) {
        if let (Some(watch), Some(session)) = (&self.watch, &self.session) {
            watch.retire_session(session.projection().stamp);
        }
    }
    fn size_command(&mut self, command: midnight_core::TreeSizeCommand) {
        use midnight_core::{TreeSizeCommand, TreeSizeEvent};
        match command {
            TreeSizeCommand::Acknowledge { stamp, version } => {
                self.sizes.acknowledge(stamp, version)
            }
            TreeSizeCommand::Cancel { stamp, request } => {
                for event in self.sizes.cancel(stamp, request) {
                    self.emit(TreeEvent::Sizes(event));
                }
            }
            TreeSizeCommand::Compute(request) => {
                let result = if !self.sizes.room() {
                    Err(ProviderError::ResourceLimited)
                } else if let Some(session) = self.session.as_mut() {
                    let cancel = &self.generation.as_ref().expect("active generation").cancel;
                    session.capture_sizes(&request, cancel)
                } else {
                    Err(ProviderError::Unavailable)
                };
                match result {
                    Ok(captured) => self.sizes.enqueue(
                        request.request,
                        captured,
                        &self.generation.as_ref().expect("active generation").cancel,
                    ),
                    Err(error) => self.emit(TreeEvent::Sizes(TreeSizeEvent::Rejected {
                        stamp: request.view.stamp,
                        request: request.request,
                        error: crate::tree_diagnostics::diagnostic(&self.reports, &error),
                    })),
                }
            }
        }
    }
    fn emit_size_selection(&mut self) {
        if !self.sizes.enabled() {
            return;
        }
        if let Some(session) = self.session.as_mut()
            && let Ok(publication) = session.publication()
        {
            self.emit(TreeEvent::Sizes(self.sizes.selected(&publication)));
        }
    }
    fn pump_sizes(&mut self) {
        let stamp = self
            .session
            .as_ref()
            .map(|session| session.projection().stamp);
        if let Some(session) = self.session.as_ref() {
            self.sizes.retain(|row| session.retains_row(row));
        }
        let events = self.sizes.tick(stamp);
        let changed = events
            .iter()
            .any(|event| matches!(event, midnight_core::TreeSizeEvent::Snapshot { .. }));
        for event in events {
            self.emit(TreeEvent::Sizes(event));
        }
        if changed {
            self.emit_size_selection();
        }
    }
}
