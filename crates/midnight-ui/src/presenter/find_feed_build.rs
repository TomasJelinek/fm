//! Feed gestures capture cheap immutable result pointers; workers build the panel.
use super::*;

pub(crate) struct PendingFeedBuild {
    request: Arc<FindFeedBuildRequest>,
    navigation: u64,
    location: String,
    roots: String,
    sort: FindSortSpec,
    sent: bool,
    ready: Option<FindFeedBuilt>,
}
impl Drop for PendingFeedBuild {
    fn drop(&mut self) {
        self.request.cancel.cancel();
    }
}
impl Presenter {
    pub(crate) fn request_feed_find_results(
        &mut self,
        id: FindSessionId,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let Some(session) = self.finds.sessions.get(&id) else {
            return false;
        };
        if session.running {
            if let Some(session) = self.finds.session_mut(&id) {
                session.error="Wait for the search to finish, or stop it, before feeding its results to a panel.".into();
            }
            return false;
        }
        let Some(work) = super::find_feed_projection::next_feed_work_id() else {
            if let Some(session) = self.finds.session_mut(&id) {
                session.error =
                    "Result view identities are exhausted. Restart the application.".into();
            }
            return false;
        };
        let pane = session.origin;
        let request = Arc::new(FindFeedBuildRequest {
            id: work,
            pane,
            session: id,
            generation: session.generation,
            results: session.results.clone().into(),
            order: session.order.clone(),
            selection_bits: session.selection.bits_snapshot(),
            explicit_marks: session.selection_is_explicit(),
            cursor: session
                .selection
                .cursor()
                .and_then(|slot| u64::try_from(slot).ok()?.checked_add(1).map(FindResultId)),
            cancel: Cancellation::default(),
        });
        let pending = PendingFeedBuild {
            request,
            navigation: self.panes[pane.index()].generation,
            location: self.panes[pane.index()].display_location_text(),
            roots: session
                .roots
                .iter()
                .filter(|root| root.searched)
                .map(|root| root.path.to_string_lossy())
                .collect::<Vec<_>>()
                .join("; "),
            sort: session.sort,
            sent: false,
            ready: None,
        };
        self.cancel_feed_build_for_pane(
            pane,
            Some("A newer Feed to panel request replaced this one."),
        );
        self.finds.feed_builds[pane.index()] = Some(pending);
        if let Some(session) = self.finds.session_mut(&id) {
            session.feeding = true;
            session.error.clear();
        }
        self.flush_find_feed_builds(send);
        true
    }
    pub(super) fn cancel_feed_build_for_pane(&mut self, pane: PaneId, reason: Option<&str>) {
        let Some(pending) = self.finds.feed_builds[pane.index()].take() else {
            return;
        };
        self.retire_feed_build(pending, reason);
    }
    fn retire_feed_build(&mut self, pending: PendingFeedBuild, reason: Option<&str>) {
        pending.request.cancel.cancel();
        if let Some(session) = self
            .finds
            .sessions
            .get_mut(&pending.request.session)
            .filter(|session| session.generation == pending.request.generation)
        {
            session.feeding = false;
            session.revision = session.revision.wrapping_add(1);
            if let Some(reason) = reason {
                session.error = reason.into();
            }
        }
        self.retired.push(Box::new(pending));
    }
    pub(super) fn cancel_feed_builds_for_session(&mut self, id: FindSessionId) {
        for pane in [PaneId::Left, PaneId::Right] {
            if self.finds.feed_builds[pane.index()]
                .as_ref()
                .is_some_and(|pending| pending.request.session == id)
            {
                self.cancel_feed_build_for_pane(pane, None);
            }
        }
    }
    fn feed_build_is_current(&self, pending: &PendingFeedBuild) -> bool {
        !pending.request.cancel.is_cancelled()
            && self
                .finds
                .sessions
                .get(&pending.request.session)
                .is_some_and(|session| {
                    session.generation == pending.request.generation
                        && session.feeding
                        && !session.running
                })
            && self.panes[pending.request.pane.index()].generation == pending.navigation
            && self.panes[pending.request.pane.index()].display_location_text() == pending.location
    }
    pub(super) fn apply_feed_build_event(&mut self, event: Event) -> Option<Event> {
        let Event::FindFeedBuilt(receipt) = event else {
            return Some(event);
        };
        let pane = receipt.request.pane;
        let current = self.finds.feed_builds[pane.index()]
            .as_ref()
            .is_some_and(|pending| {
                pending.request.id == receipt.request.id
                    && pending.request.session == receipt.request.session
                    && pending.request.generation == receipt.request.generation
            });
        if !current {
            self.retired.push(receipt);
            return None;
        }
        let valid = self.finds.feed_builds[pane.index()]
            .as_ref()
            .is_some_and(|pending| self.feed_build_is_current(pending));
        if !valid {
            self.cancel_feed_build_for_pane(
                pane,
                Some("The search or panel changed. Feed the current results again."),
            );
            self.retired.push(receipt);
            return None;
        }
        let FindFeedBuildResult { request, result } = *receipt;
        match result {
            Ok(built) => {
                if let Some(pending) = &mut self.finds.feed_builds[pane.index()] {
                    pending.ready = Some(built);
                }
                if let Some(session) = self.finds.session_mut(&request.session) {
                    session.error.clear();
                }
            }
            Err(error) => {
                self.cancel_feed_build_for_pane(pane, None);
                if let Some(session) = self.finds.session_mut(&request.session) {
                    session.error = crate::error_presentation::describe(&error);
                }
                self.retired.push(Box::new(error));
            }
        }
        None
    }
    pub(super) fn flush_find_feed_builds(&mut self, send: &dyn Fn(Command) -> bool) {
        for pane in [PaneId::Left, PaneId::Right] {
            let Some(mut pending) = self.finds.feed_builds[pane.index()].take() else {
                continue;
            };
            if !self.feed_build_is_current(&pending) {
                self.retire_feed_build(
                    pending,
                    Some("The search or panel changed. Feed the current results again."),
                );
                continue;
            }
            if pending.ready.is_some() {
                if self.close_find(pending.request.session, send) {
                    if let Some(built) = pending.ready.take() {
                        let title = format!(
                            "Search results ({}) · {}",
                            self.number_locale.format_u64(built.entries.len() as u64),
                            pending.roots
                        );
                        self.install_built_feed(
                            pane,
                            built,
                            title,
                            pending.location.clone(),
                            pending.sort,
                        );
                        self.finds.feed_focus.set(Some(pane));
                    }
                    self.retired.push(Box::new(pending));
                } else {
                    self.finds.feed_builds[pane.index()] = Some(pending);
                }
                continue;
            }
            if !pending.sent {
                pending.sent = send(Command::BuildFindFeed(pending.request.clone()));
                if !pending.sent
                    && let Some(session) = self.finds.session_mut(&pending.request.session)
                {
                    session.error =
                        "Result preparation queue is busy. Waiting for capacity…".into();
                }
            }
            self.finds.feed_builds[pane.index()] = Some(pending);
        }
    }
}

#[cfg(test)]
#[path = "find_feed_build_tests.rs"]
mod tests;
