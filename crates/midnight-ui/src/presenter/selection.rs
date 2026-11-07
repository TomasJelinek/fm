//! Selection commands carry only shared snapshots and single raw path intents.
use super::*;

impl Presenter {
    pub(crate) fn bulk_selection(
        &mut self,
        pane: PaneId,
        operation: SelectionOperation,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        if self.panes[pane.index()].archive.is_some() {
            return self.archive_bulk_selection(
                pane,
                ArchiveSelectionAction::Bulk(operation),
                send,
            );
        }
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
        if self.panes[pane.index()].archive.is_some() {
            return false;
        }
        if !matches!(action, SelectionAction::Rebase)
            && self.panes[pane.index()].search_view_waiting()
        {
            return false;
        }
        let p = &mut self.panes[pane.index()];
        let Some(revision) = p.selection_revision.0.checked_add(1).map(SelectionRevision) else {
            p.error = "Selection request identities are exhausted.".into();
            return false;
        };
        let rebase = matches!(&action, SelectionAction::Rebase);
        if !rebase && p.selection_preview.len() == selection_preview::PREVIEW_CAPACITY {
            p.error = "Selection queue is busy. Try again.".into();
            return false;
        }
        let preview = (!rebase)
            .then(|| selection_preview::PendingSelection::new(p, revision, action.clone()));
        let toggle_directory = if let SelectionAction::Toggle(path) = &action {
            p.display_index()
                .by_path
                .get(path)
                .and_then(|id| p.entry_by_id(*id))
                .or_else(|| {
                    p.cursor
                        .and_then(|id| p.entry_by_id(id))
                        .filter(|entry| entry.path == *path)
                })
                .is_some_and(|entry| {
                    matches!(entry.kind, EntryKind::Directory | EntryKind::DirectoryLink)
                })
        } else {
            false
        };
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
            seed_revision: p.selection_applied,
            authoritative: self.comparison.seeds[pane.index()].clone(),
            reset: p.selection_reset,
            action,
            toggle_directory,
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
        if let Some(preview) = preview {
            p.selection_preview.push(preview);
        }
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
        self.flush_selection_sizes(send);
    }

    pub(super) fn selection_navigation_accepted(
        &mut self,
        pane: PaneId,
        navigation: NavigationId,
        path: &Path,
    ) {
        self.cancel_selection_actions(pane);
        let p = &mut self.panes[pane.index()];
        p.selection_preview.clear();
        if path != p.path {
            p.selection_anchor = None;
            p.selection_anchor_id = None;
        }
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
                        .push(Box::new(self.comparison.seeds[pane.index()].take()));
                    self.retired
                        .push(Box::new(mem::replace(&mut p.selected, selection)));
                    p.selection_applied = revision;
                    p.selection_preview
                        .retain(|pending| pending.revision > revision);
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
                if navigation.0 == p.generation
                    && revision <= p.selection_revision
                    && revision > p.selection_applied
                    && revision > p.selection_resolved
                {
                    p.selection_preview
                        .retain(|pending| pending.revision > revision);
                    p.selection_resolved = p.selection_resolved.max(revision);
                    p.error = crate::error_presentation::describe(&error);
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
        self.request_selection_action(pane, selection_actions::SelectionIntent::Properties, send);
    }
}
