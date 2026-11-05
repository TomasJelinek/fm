//! Selection commands carry only shared snapshots and single raw path intents.
use super::*;

pub(super) struct PendingProperties {
    request: DesktopRequestId,
    navigation: NavigationId,
    revision: SelectionRevision,
    view: Option<Arc<DirectoryIndex>>,
    fallback: Option<PathBuf>,
}

impl Presenter {
    pub(crate) fn bulk_selection(
        &mut self,
        pane: PaneId,
        operation: SelectionOperation,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        if self.panes[pane.index()].search_view_waiting() {
            return false;
        }
        self.send_selection(pane, SelectionAction::Bulk(operation), send)
    }

    pub(super) fn send_selection(
        &mut self,
        pane: PaneId,
        action: SelectionAction,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let p = &mut self.panes[pane.index()];
        let Some(revision) = p.selection_revision.0.checked_add(1).map(SelectionRevision) else {
            p.error = "Selection request identities are exhausted.".into();
            return false;
        };
        let rebase = matches!(&action, SelectionAction::Rebase);
        let pending_bulk = match &action {
            SelectionAction::Bulk(SelectionOperation::All | SelectionOperation::Invert) => {
                Some(true)
            }
            SelectionAction::Bulk(SelectionOperation::Clear) => Some(false),
            _ => None,
        };
        let request = SelectionRequest {
            pane,
            navigation: NavigationId(p.generation),
            revision,
            context: p.selection_context,
            location: p.display_path().to_owned(),
            view: (!p.showing_stream()).then(|| p.display_index().clone()),
            seed: p.selected.clone(),
            reset: p.selection_reset,
            action,
            preserve: p
                .pending
                .as_ref()
                .and_then(|pending| pending.previous_selection_origin.clone()),
            restore: p.selection_restore.clone(),
        };
        if !send(Command::Selection(request)) {
            p.error = "Selection queue is busy. Try again.".into();
            return false;
        }
        p.selection_revision = revision;
        p.selection_reset = false;
        p.selection_restore = None;
        if p.showing_stream()
            && let Some(pending) = pending_bulk
        {
            p.selection_pending_bulk = pending;
        }
        if !rebase {
            p.selection_dirty = true;
        }
        true
    }

    /// Retry internal rebases after bounded command-channel backpressure.
    pub fn flush_selection(&mut self, send: &dyn Fn(Command) -> bool) {
        for pane in [PaneId::Left, PaneId::Right] {
            if self.panes[pane.index()].selection_rebase_pending
                && !self.panes[pane.index()].loading
                && !self.panes[pane.index()].search_view_waiting()
                && self.send_selection(pane, SelectionAction::Rebase, send)
            {
                self.panes[pane.index()].selection_rebase_pending = false;
            }
        }
    }

    pub(super) fn selection_navigation_accepted(
        &mut self,
        pane: PaneId,
        navigation: NavigationId,
        path: &Path,
    ) {
        self.cancel_selection_properties(pane);
        let p = &mut self.panes[pane.index()];
        p.selection_context = if path == p.path && p.successful_generation.is_some() {
            p.selection_committed_context
        } else {
            navigation
        };
        p.selection_revision = SelectionRevision(0);
        p.selection_applied = SelectionRevision(0);
        p.selection_resolved = SelectionRevision(0);
        p.selection_reset = path != p.path || p.successful_generation.is_none();
        p.selection_restore = None;
        p.selection_pending_bulk = false;
        p.selection_dirty = false;
        p.selection_rebase_pending = false;
        p.selection_deferred = false;
    }

    pub(super) fn apply_selection_event(&mut self, event: Event) -> Option<Event> {
        match event {
            Event::SelectionChanged {
                pane,
                navigation,
                revision,
                selection,
                deferred,
            } => {
                let p = &mut self.panes[pane.index()];
                if navigation.0 == p.generation
                    && revision <= p.selection_revision
                    && revision > p.selection_applied
                {
                    self.retired
                        .push(Box::new(mem::replace(&mut p.selected, selection)));
                    p.selection_applied = revision;
                    p.selection_resolved = p.selection_resolved.max(revision);
                    p.selection_deferred = deferred;
                    if !deferred && !p.showing_stream() {
                        p.selection_pending_bulk = false;
                    }
                } else {
                    self.retired.push(Box::new(selection));
                }
                None
            }
            Event::SelectionFailed {
                pane,
                navigation,
                revision,
                error,
            } => {
                let p = &mut self.panes[pane.index()];
                if navigation.0 == p.generation && revision <= p.selection_revision {
                    p.selection_resolved = p.selection_resolved.max(revision);
                    p.error = error.to_string();
                    p.selection_deferred = false;
                    p.selection_rebase_pending = true;
                }
                None
            }
            event => Some(event),
        }
    }

    pub(super) fn request_selection_properties(
        &mut self,
        pane: PaneId,
        send: &dyn Fn(Command) -> bool,
    ) {
        if !self.panes[pane.index()].search_allows_activation() {
            return;
        }
        if self.selection_properties[pane.index()].is_some() {
            self.desktop.report(
                "Properties waiting",
                "A Properties request is already waiting for selection.".into(),
            );
            return;
        }
        let p = &self.panes[pane.index()];
        if p.selection_pending_bulk || p.selection_deferred || p.selection_rebase_pending {
            self.desktop.report(
                "Selection still loading",
                "Wait for the folder listing and selection to finish before opening Properties."
                    .into(),
            );
            return;
        }
        let fallback = (!p.parent_cursor)
            .then(|| {
                p.cursor
                    .and_then(|id| p.entry_by_id(id))
                    .map(|entry| entry.path.clone())
            })
            .flatten();
        // A streaming single-row fallback is tiny. Multi-selection expansion is
        // always deferred to the backend, using a shared stable view.
        let view = (!p.showing_stream()).then(|| p.display_index().clone());
        let pending = p.selection_revision > p.selection_resolved;
        let selection = p.selected.clone();
        let navigation = NavigationId(p.generation);
        let revision = p.selection_revision;
        if !pending && selection.is_empty() && fallback.is_none() {
            return;
        }
        let request = self.desktop.request();
        if pending {
            self.desktop.requests.insert(request);
            self.selection_properties[pane.index()] = Some(PendingProperties {
                request,
                navigation,
                revision,
                view,
                fallback,
            });
        } else if send(Command::ShowSelectionProperties {
            request,
            view: view.unwrap_or_else(|| selection.basis().clone()),
            selection,
            fallback,
        }) {
            self.desktop.requests.insert(request);
        } else {
            self.desktop.report(
                "Properties queue busy",
                "Try opening Properties again.".into(),
            );
        }
    }

    pub(super) fn dispatch_selection_properties(
        &mut self,
        event: &Event,
        send: &dyn Fn(Command) -> bool,
    ) {
        match event {
            Event::SelectionChanged {
                pane,
                navigation,
                revision,
                selection,
                ..
            } => {
                let slot = pane.index();
                let matches = self.selection_properties[slot]
                    .as_ref()
                    .is_some_and(|pending| {
                        pending.navigation == *navigation
                            && pending.revision == *revision
                            && self.panes[slot].generation == navigation.0
                    });
                if !matches {
                    return;
                }
                let pending = self.selection_properties[slot]
                    .take()
                    .expect("matched pending Properties");
                let request = pending.request;
                if !send(Command::ShowSelectionProperties {
                    request,
                    selection: selection.clone(),
                    view: pending
                        .view
                        .clone()
                        .unwrap_or_else(|| selection.basis().clone()),
                    fallback: pending.fallback.clone(),
                }) {
                    self.desktop.requests.remove(&request);
                    self.desktop.report(
                        "Properties queue busy",
                        "Try opening Properties again.".into(),
                    );
                }
                self.retired.push(Box::new(pending));
            }
            Event::SelectionFailed {
                pane,
                navigation,
                revision,
                ..
            } if self.selection_properties[pane.index()]
                .as_ref()
                .is_some_and(|pending| {
                    pending.navigation == *navigation && pending.revision <= *revision
                }) =>
            {
                self.cancel_selection_properties(*pane);
                self.desktop.report(
                    "Properties cancelled",
                    "Selection could not finish. Try opening Properties again.".into(),
                );
            }
            _ => {}
        }
    }

    pub(super) fn cancel_selection_properties(&mut self, pane: PaneId) {
        if let Some(pending) = self.selection_properties[pane.index()].take() {
            self.desktop.requests.remove(&pending.request);
            self.retired.push(Box::new(pending));
        }
    }

    pub(super) fn retire_selection_properties(&mut self) {
        self.cancel_selection_properties(PaneId::Left);
        self.cancel_selection_properties(PaneId::Right);
    }
}
