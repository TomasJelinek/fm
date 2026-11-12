//! Exact backend tree publications and bounded presentation intents.
use super::*;

struct PendingEnable {
    root: TreeRootSnapshot,
    entry: EntryId,
    sent: Option<SelectionRevision>,
}

#[derive(Default)]
pub(super) struct State {
    pub(super) publication: Option<TreePublication>,
    pending: Option<PendingEnable>,
    pub(super) commit_pending: bool,
    expand_after_commit: Option<TreeRowId>,
    pub(super) request: TreeRequestId,
    pub(super) options: tree_options::State,
    pub(super) sizes: tree_sizes::State,
    pub(super) refresh: tree_refresh::State,
    navigation: Option<(TreeViewStamp, TreeRequestId, TreeRowId, NavigationId)>,
    pub(super) revision: SelectionRevision,
    pub(super) applied: SelectionRevision,
    /// Highest retired selection request; rejected requests do not change applied authority.
    pub(super) resolved: SelectionRevision,
    pub(super) search_cursor: Option<TreeRowId>,
    pub(super) search_insert: Option<tree_search::Insert>,
    pub(super) preview: Vec<(SelectionRevision, TreeSelectionAction)>,
    replay: std::collections::VecDeque<(SelectionRevision, TreeSelectionAction)>,
    pub(super) anchor: Option<TreeRowId>,
    pub(super) viewport: (usize, usize),
    viewport_sent: Option<(TreeRevision, usize, usize)>,
}

impl Pane {
    pub(super) fn tree_input_ready(&self) -> bool {
        self.tree_projection_ready() && !self.search.pending
    }

    pub(super) fn tree_projection_ready(&self) -> bool {
        !self.tree.commit_pending
            && self.tree.replay.is_empty()
            && self.tree.navigation.is_none()
            && !self.tree.options.is_pending()
    }
    pub(crate) fn set_tree_viewport(&mut self, range: (usize, usize)) {
        self.tree.viewport = range;
    }
    pub(crate) fn tree_view(&self) -> Option<&Arc<TreeProjection>> {
        self.tree.publication.as_ref().map(|value| &value.view)
    }

    pub(crate) fn tree_row(&self, position: usize) -> Option<&TreeDisplayRow> {
        self.tree_view()?.rows.get(position)
    }

    pub(crate) fn row_identity(&self, entry: &Entry) -> String {
        if let Some(view) = self.tree_view() {
            format!(
                "{}:{}:tree:{}:{}:{}",
                self.generation,
                view.stamp.navigation.0,
                view.stamp.session.0,
                view.stamp.revision.0,
                entry.id.0
            )
        } else {
            format!(
                "{}:{}:{}:entry:{}",
                self.generation,
                self.displayed_generation().0,
                self.search.view_revision,
                entry.id.0
            )
        }
    }

    pub(crate) fn root_disclosure_available(&self) -> bool {
        !self.loading
            && self.network.is_none()
            && !self.network_request
            && self.search.query.is_empty()
            && !self.search.panel_filter.enabled
            && self.successful_generation == Some(NavigationId(self.generation))
    }

    fn root_selection_revision(&self) -> SelectionRevision {
        self.archive
            .as_ref()
            .map_or(self.selection_revision, |a| a.selection_revision)
    }

    fn root_selection_ready(&self) -> bool {
        self.archive.as_ref().map_or_else(
            || {
                self.selection_revision == self.selection_resolved
                    && self.selection_preview.is_empty()
                    && !self.selection_rebase_pending
                    && !self.selection_deferred.any()
                    && (self.selected.is_empty() || Arc::ptr_eq(self.selected.basis(), &self.index))
            },
            |a| !a.selection_pending,
        )
    }
}

impl Presenter {
    /// Retire presentation before a location transition; restore only an exact root alias.
    pub(super) fn clear_tree(&mut self, pane: PaneId, navigation: Option<NavigationId>) {
        self.clear_folder_presence(pane);
        self.cancel_tree_transfer(Some(pane));
        self.cancel_tree_clipboard(Some(pane));
        self.cancel_tree_editor(Some(pane));
        self.cancel_tree_checksums(Some(pane));
        self.cancel_tree_delete(Some(pane));
        self.cancel_tree_drag(Some(pane));
        if self.filter_dialog.open && self.filter_dialog.pane == pane {
            self.cancel_panel_filter();
        }
        let p = &mut self.panes[pane.index()];
        if let Some(view) = p.tree_view() {
            p.cursor = p
                .cursor
                .and_then(|entry| view.index.directory.by_id.get(&entry))
                .and_then(|position| view.rows.get(*position))
                .and_then(|row| row.original_root_entry)
                .and_then(|alias| view.root_actions.index.by_id.get(&alias))
                .and_then(|position| view.root_actions.entries.get(*position))
                .and_then(|entry| {
                    if entry.virtual_node.is_some() {
                        view.root
                            .index
                            .by_id
                            .contains_key(&entry.id)
                            .then_some(entry.id)
                    } else {
                        view.root.index.by_path.get(&entry.path).copied()
                    }
                });
            p.parent_cursor |= p.cursor.is_none() && p.has_parent();
            // The old flat marks must not reappear after a tree-only deselection.
            // Navigation establishes a fresh selection scope through the normal path.
            self.retired.push(Box::new(mem::take(&mut p.selected)));
            if let Some(navigation) = navigation {
                // A fresh backend context is required even for same-root refresh
                // and failed-navigation restoration. An empty UI seed alone does
                // not replace the previous flat worker's accepted selection.
                p.selection_context = navigation;
                p.selection_committed_context = navigation;
                p.selection_revision = SelectionRevision(0);
                p.selection_applied = SelectionRevision(0);
                p.selection_resolved = SelectionRevision(0);
                p.selection_reset = true;
            }
            if let Some(archive) = &mut p.archive {
                archive.clear_tree_selection(&mut self.retired);
            }
        }
        self.retired.push(Box::new(mem::take(&mut p.tree)));
        self.retired.push(Box::new(p.error_details.take()));
        p.rectangle_preview = None;
    }

    /// Temporary typed-action boundary. A display index never grants native operation authority.
    pub(super) fn tree_action_allowed(&mut self, pane: PaneId) -> bool {
        if self.panes[pane.index()].tree_view().is_none() {
            return true;
        }
        self.desktop.report("Inline branch action unavailable",
            "Open the folder with Enter before using this action. It is not available on inline branches yet.".into());
        false
    }

    pub(crate) fn disclose_tree(
        &mut self,
        pane: PaneId,
        identity: &str,
        send: &dyn Fn(Command) -> bool,
    ) {
        let p = &self.panes[pane.index()];
        if p.loading || p.network.is_some() || p.network_request {
            return;
        }
        if let Some(view) = p.tree_view() {
            let Some(id) = identity
                .rsplit(':')
                .next()
                .and_then(|id| id.parse::<u64>().ok())
                .map(TreeRowId)
            else {
                return;
            };
            let Some(row) = view
                .index
                .row_positions
                .get(&id)
                .and_then(|position| view.rows.get(*position))
            else {
                return;
            };
            if identity != p.row_identity(&row.entry) {
                return;
            }
            let action = if row.expanded {
                TreeBranchAction::Collapse
            } else {
                match row.disclosure {
                    TreeDisclosure::Unknown
                    | TreeDisclosure::Collapsed
                    | TreeDisclosure::Failed => TreeBranchAction::Expand,
                    _ => return,
                }
            };
            self.send_tree_branch(pane, id, action, send);
            return;
        }
        if !p.root_disclosure_available() {
            return;
        }
        let Some(entry) = identity
            .rsplit(':')
            .next()
            .and_then(|id| id.parse::<u64>().ok())
            .map(EntryId)
            .and_then(|id| p.index.by_id.get(&id))
            .and_then(|position| p.entries.get(*position))
        else {
            return;
        };
        if entry.kind != EntryKind::Directory || identity != p.row_identity(entry) {
            return;
        }
        let pending = PendingEnable {
            root: TreeRootSnapshot {
                pane,
                navigation: NavigationId(p.generation),
                entries: p.entries.clone(),
                index: p.index.clone(),
            },
            entry: entry.id,
            sent: None,
        };
        self.retired.push(Box::new(
            self.panes[pane.index()].tree.pending.replace(pending),
        ));
        self.flush_trees(send);
    }

    fn send_tree_branch(
        &mut self,
        pane: PaneId,
        row: TreeRowId,
        action: TreeBranchAction,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let p = &mut self.panes[pane.index()];
        if !p.tree_input_ready() {
            return false;
        }
        let Some(view) = p.tree_view().cloned() else {
            return false;
        };
        let Some(request) = p.tree.request.checked_next() else {
            return false;
        };
        if !send(Command::Tree(Box::new(TreeCommand::Branch {
            view,
            request,
            row,
            action,
        }))) {
            p.set_error("Branch queue is busy. Try again.", &mut self.retired);
            return false;
        }
        p.tree.request = request;
        true
    }

    pub(super) fn flush_trees(&mut self, send: &dyn Fn(Command) -> bool) {
        self.flush_tree_size_acks(send);
        for pane in [PaneId::Left, PaneId::Right] {
            let p = &mut self.panes[pane.index()];
            if p.loading {
                continue;
            }
            if p.tree.publication.is_none()
                && p.root_selection_ready()
                && p.root_disclosure_available()
            {
                let revision = p.root_selection_revision();
                if let Some(pending) = &mut p.tree.pending
                    && pending.sent != Some(revision)
                {
                    let selected = p.archive.as_ref().map_or_else(
                        || {
                            if p.selected.is_empty() {
                                TreeRootSelection::Empty
                            } else {
                                TreeRootSelection::Native(p.selected.clone())
                            }
                        },
                        |a| TreeRootSelection::Archive(a.selected.clone()),
                    );
                    if send(Command::Tree(Box::new(TreeCommand::Enable {
                        root: pending.root.clone(),
                        selection_revision: revision,
                        selected,
                    }))) {
                        pending.sent = Some(revision);
                    }
                }
            }
            let Some(view) = p.tree_view().cloned() else {
                continue;
            };
            if p.tree.commit_pending {
                if !send(Command::Tree(Box::new(TreeCommand::Commit(view.clone())))) {
                    continue;
                }
                p.tree.commit_pending = false;
            }
            if p.tree.options.is_pending() {
                continue;
            }
            while let Some((revision, action)) = p.tree.replay.front() {
                let publication = p.tree.publication.as_ref().expect("published view");
                let request = TreeSelectionRequest {
                    view: view.clone(),
                    revision: *revision,
                    seed: Some(publication.selection.clone()),
                    seed_revision: publication.selection_revision,
                    action: action.clone(),
                };
                if !send(Command::Tree(Box::new(TreeCommand::Select(request)))) {
                    break;
                }
                p.tree.replay.pop_front();
            }
            if !p.tree.replay.is_empty() {
                continue;
            }
            if let Some(row) = p.tree.expand_after_commit {
                if self.send_tree_branch(pane, row, TreeBranchAction::Expand, send) {
                    self.panes[pane.index()].tree.expand_after_commit = None;
                }
                // Do not let a probe publication overtake the explicitly requested expansion.
                continue;
            }
            if self.flush_tree_search_insert(pane, send) {
                continue;
            }
            self.flush_tree_sizes(pane, send);
            let p = &mut self.panes[pane.index()];
            if !p.tree.preview.is_empty() {
                continue;
            }
            let (first, end) = p.tree.viewport;
            let first = first.saturating_sub(p.parent_offset()).min(view.rows.len());
            let end = end
                .saturating_sub(p.parent_offset())
                .min(view.rows.len())
                .min(first.saturating_add(TreeViewport::MAX_ROWS));
            let key = (view.stamp.revision, first, end);
            if p.tree.viewport_sent == Some(key) || first >= end {
                continue;
            }
            let ids = view.rows[first..end].iter().map(|row| row.id).collect();
            if let Ok(rows) = TreeViewport::new(&view, ids)
                && send(Command::Tree(Box::new(TreeCommand::Viewport {
                    view,
                    rows,
                })))
            {
                p.tree.viewport_sent = Some(key);
            }
        }
        self.flush_tree_transfer(send);
        self.flush_tree_clipboard(send);
        self.flush_tree_editor(send);
        self.flush_tree_checksums(send);
        self.flush_tree_delete(send);
        self.flush_tree_drag(send);
    }

    pub(super) fn open_tree_row(
        &mut self,
        pane: PaneId,
        position: usize,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let p = &mut self.panes[pane.index()];
        if p.loading || !p.tree_input_ready() {
            return false;
        }
        let Some(view) = p.tree_view().cloned() else {
            return false;
        };
        let Some(row) = position
            .checked_sub(p.parent_offset())
            .and_then(|position| view.rows.get(position))
        else {
            return false;
        };
        if !row.navigable {
            // Only the exact retained root alias may use the established flat-file
            // activation path. Occurrence IDs and descendant paths are never substituted.
            let original = row
                .original_root_entry
                .and_then(|id| view.root_actions.index.by_id.get(&id))
                .and_then(|position| view.root_actions.entries.get(*position));
            if let Some(original) = original {
                if original.virtual_node.is_some() {
                    return self.archive_open(pane, position, send);
                }
                let path = original.path.clone();
                let kind = original.kind;
                let archive = self.browsable_archive_format(original);
                if matches!(kind, EntryKind::File | EntryKind::Symlink) {
                    if let Some(format) = archive {
                        return self.archive_navigate(
                            pane,
                            ArchiveAction::Open { path, format },
                            send,
                        );
                    }
                    return self.open_native_entry(pane, path, send);
                }
            }
            self.tree_action_allowed(pane);
            return false;
        }
        let (Some(request), Some(navigation)) = (
            p.tree.request.checked_next(),
            p.generation.checked_add(1).map(NavigationId),
        ) else {
            return false;
        };
        let row = row.id;
        let stamp = view.stamp;
        if !send(Command::Tree(Box::new(TreeCommand::Navigate {
            view,
            request,
            row,
            navigation,
        }))) {
            return false;
        }
        p.tree.request = request;
        p.tree.navigation = Some((stamp, request, row, navigation));
        true
    }

    pub(super) fn apply_tree_event(&mut self, event: TreeEvent, send: &dyn Fn(Command) -> bool) {
        // Even stale/rejected terminal receipts may own the last private report.
        // Retain one owner for backend reclamation before individual arms consume it.
        let diagnostic = match &event {
            TreeEvent::WatchFailed { error, .. }
            | TreeEvent::RefreshRejected { error, .. }
            | TreeEvent::ReprojectFailed { error, .. }
            | TreeEvent::Unavailable { error, .. }
            | TreeEvent::Rejected { error, .. }
            | TreeEvent::SelectionFailed { error, .. }
            | TreeEvent::Sizes(TreeSizeEvent::Rejected { error, .. }) => Some(error),
            TreeEvent::Refreshed {
                result: Err(error), ..
            }
            | TreeEvent::Transfer {
                result: Err(error), ..
            }
            | TreeEvent::Clipboard {
                result: Err(error), ..
            }
            | TreeEvent::EditorCaptured {
                result: Err(error), ..
            }
            | TreeEvent::ChecksumsCaptured {
                result: Err(error), ..
            }
            | TreeEvent::DeleteCaptured {
                result: Err(error), ..
            }
            | TreeEvent::DropDirectoryCaptured {
                result: Err(error), ..
            } => Some(error),
            _ => None,
        };
        if let Some(content) = diagnostic.and_then(TreeDiagnostic::details) {
            self.retired.push(Box::new(content.clone()));
        }
        match event {
            TreeEvent::DeleteCaptured {
                stamp,
                request,
                result,
            } => {
                self.tree_delete_captured(stamp, request, result, send);
            }
            TreeEvent::ChecksumsCaptured {
                stamp,
                request,
                result,
            } => {
                self.tree_checksums_captured(stamp, request, result, send);
            }
            TreeEvent::EditorCaptured {
                stamp,
                request,
                result,
            } => {
                self.tree_editor_captured(stamp, request, result, send);
            }
            TreeEvent::DropDirectoryCaptured {
                stamp,
                request,
                id,
                result,
            } => {
                self.tree_drop_captured(stamp, request, id, result, send);
            }
            TreeEvent::Changed { stamp } => {
                if self.panes[stamp.pane.index()]
                    .tree_view()
                    .is_some_and(|view| {
                        view.stamp.navigation == stamp.navigation
                            && view.stamp.session == stamp.session
                    })
                {
                    self.request_native_tree_refresh(stamp.pane, send);
                }
            }
            TreeEvent::WatchFailed { stamp, error } => {
                let p = &mut self.panes[stamp.pane.index()];
                if p.tree_view().is_some_and(|view| {
                    view.stamp.navigation == stamp.navigation && view.stamp.session == stamp.session
                }) {
                    p.set_tree_error(
                        &error,
                        "Could not watch every expanded folder: ",
                        &mut self.retired,
                    );
                }
            }
            TreeEvent::Refreshed {
                stamp,
                request,
                result,
            } => {
                self.tree_refresh_finished(stamp, request, result, false);
            }
            TreeEvent::RefreshRejected {
                stamp,
                request,
                error,
                retryable,
            } => {
                self.tree_refresh_finished(stamp, request, Err(error), retryable);
            }
            TreeEvent::Clipboard {
                stamp,
                request,
                result,
            } => {
                if self.matches_tree_drag_capture(stamp, request) {
                    self.tree_drag_captured(result, send);
                } else {
                    self.tree_clipboard_captured(stamp, request, result, send);
                }
            }
            TreeEvent::Transfer {
                stamp,
                request,
                result,
            } => {
                self.tree_transfer_captured(stamp, request, result);
            }
            TreeEvent::Sizes(event) => self.apply_tree_sizes(event, send),
            TreeEvent::Reprojected {
                previous,
                request,
                options,
                publication,
            } => {
                self.tree_options_applied(previous, request, options, publication, send);
            }
            TreeEvent::ReprojectFailed {
                stamp,
                request,
                error,
                retryable,
            } => {
                self.tree_options_failed(stamp, request, error, retryable);
            }
            TreeEvent::Published(publication) => {
                self.publish_tree(publication);
            }
            TreeEvent::SelectionChanged {
                view,
                revision,
                selection,
            } => {
                let p = &mut self.panes[view.stamp.pane.index()];
                if p.generation == view.stamp.navigation.0
                    && p.tree_view()
                        .is_some_and(|current| Arc::ptr_eq(current, &view))
                    && Arc::ptr_eq(&view, selection.basis())
                    && revision <= p.tree.revision
                    && revision > p.tree.applied
                {
                    let published = p.tree.publication.as_mut().expect("validated tree view");
                    self.retired
                        .push(Box::new(mem::replace(&mut published.selection, selection)));
                    published.selection_revision = revision;
                    p.tree.applied = revision;
                    p.tree.resolved = p.tree.resolved.max(revision);
                    p.tree.preview.retain(|(pending, _)| *pending > revision);
                    self.tree_copy_selection_result(view.stamp, revision, true);
                } else {
                    self.retired.push(Box::new(selection));
                }
                self.retired.push(Box::new(view));
            }
            TreeEvent::SelectionFailed {
                stamp,
                revision,
                error,
            } => {
                self.file_list_selection_failed(stamp.pane, stamp.navigation, revision);
                self.tree_copy_selection_result(stamp, revision, false);
                let p = &mut self.panes[stamp.pane.index()];
                if p.tree_view().is_some_and(|view| view.stamp == stamp)
                    && revision > p.tree.applied
                    && revision <= p.tree.revision
                {
                    p.tree.preview.retain(|(pending, _)| *pending != revision);
                    p.tree.resolved = p.tree.resolved.max(revision);
                    p.set_tree_error(&error, "", &mut self.retired);
                }
            }
            TreeEvent::Navigated {
                stamp,
                request,
                row,
                navigation,
                target,
            } => {
                let pane = stamp.pane;
                let p = &mut self.panes[pane.index()];
                let valid = p.generation == stamp.navigation.0
                    && p.tree.navigation == Some((stamp, request, row, navigation))
                    && p.tree_view().is_some_and(|view| view.stamp == stamp)
                    && p.generation.checked_add(1) == Some(navigation.0);
                if valid {
                    p.tree.navigation = None;
                    match &target {
                        TreeNavigationTarget::Native(path) => {
                            let link = p
                                .tree_view()
                                .and_then(|view| {
                                    view.index.row_positions.get(&row).map(|position| {
                                        view.rows[*position].entry.kind == EntryKind::DirectoryLink
                                    })
                                })
                                .unwrap_or(false);
                            if link {
                                self.open_native_directory(pane, path.clone(), send);
                            } else {
                                self.navigate(pane, path.clone(), None, send);
                            }
                        }
                        TreeNavigationTarget::Archive(location) => {
                            self.archive_navigate(
                                pane,
                                ArchiveAction::List(location.clone()),
                                send,
                            );
                        }
                    }
                }
                self.retired.push(Box::new(target));
            }
            TreeEvent::Unavailable { root, error } => {
                let p = &mut self.panes[root.pane.index()];
                if p.tree
                    .pending
                    .as_ref()
                    .is_some_and(|pending| pending.root.same_snapshot(&root))
                {
                    self.retired.push(Box::new(p.tree.pending.take()));
                    p.set_tree_error(&error, "", &mut self.retired);
                }
                self.retired.push(Box::new(root));
            }
            TreeEvent::Rejected {
                stamp,
                request,
                error,
                ..
            } => {
                let p = &mut self.panes[stamp.pane.index()];
                if p.tree_view().is_some_and(|view| view.stamp == stamp) {
                    if request == TreeRequestId(0) {
                        p.tree.commit_pending = true;
                        p.tree.viewport_sent = None;
                    }
                    if p.tree
                        .navigation
                        .as_ref()
                        .is_some_and(|(_, pending, _, _)| *pending == request)
                    {
                        p.tree.navigation = None;
                    }
                    p.set_tree_error(&error, "", &mut self.retired);
                }
            }
        }
        self.flush_trees(send);
        self.flush_tree_preferences(send);
    }

    pub(super) fn publish_tree(&mut self, publication: TreePublication) -> bool {
        let pane = publication.view.stamp.pane;
        let p = &mut self.panes[pane.index()];
        let root = &publication.view.root;
        if p.loading
            || p.generation != root.navigation.0
            || !Arc::ptr_eq(&p.entries, &root.entries)
            || !Arc::ptr_eq(&p.index, &root.index)
            || !Arc::ptr_eq(&publication.view, publication.selection.basis())
        {
            self.retired.push(Box::new(publication));
            return false;
        }
        let search_cursor = p
            .tree_cursor_row()
            .filter(|row| row.selectable && !row.context_only)
            .map(|row| row.id);
        let previous = p.tree_view();
        if let Some(previous) = previous {
            if previous.stamp.session != publication.view.stamp.session
                || previous.stamp.revision.0 >= publication.view.stamp.revision.0
            {
                self.retired.push(Box::new(publication));
                return false;
            }
        } else {
            if !p.root_disclosure_available() {
                self.retired.push(Box::new(p.tree.pending.take()));
                self.retired.push(Box::new(publication));
                return false;
            }
            let Some(pending) = &p.tree.pending else {
                self.retired.push(Box::new(publication));
                return false;
            };
            if !p.root_selection_ready()
                || !publication.can_adopt(&pending.root, p.root_selection_revision())
            {
                p.tree.pending.as_mut().expect("pending adoption").sent = None;
                self.retired.push(Box::new(publication));
                return false;
            }
        }
        let view = &publication.view;
        let same_layout =
            previous.is_some_and(|previous| previous.layout_revision == view.layout_revision);
        let mut focused = p.cursor.map(|id| TreeRowId(id.0));
        if let Some(previous) = previous {
            while let Some(id) = focused {
                if view.index.row_positions.contains_key(&id) {
                    break;
                }
                focused = previous
                    .index
                    .row_positions
                    .get(&id)
                    .and_then(|position| previous.rows.get(*position))
                    .and_then(|row| row.parent);
            }
        } else {
            focused = p
                .cursor
                .and_then(|id| view.index.root_rows.get(&id).copied());
            p.tree.expand_after_commit = p
                .tree
                .pending
                .as_ref()
                .and_then(|pending| view.index.root_rows.get(&pending.entry).copied());
        }
        p.cursor = focused
            .and_then(|id| view.index.row_positions.get(&id))
            .and_then(|position| view.rows.get(*position))
            .map(|row| row.entry.id);
        if let Some(cursor) = search_cursor {
            p.tree.search_cursor = Some(cursor);
        }
        if p.tree
            .search_insert
            .is_some_and(|insert| !insert.visible_in(view))
        {
            p.tree.search_insert = None;
        }
        p.parent_cursor |= p.cursor.is_none() && p.has_parent();
        p.preserve_viewport = true;
        p.rectangle_preview = None;
        // Preserve bounded pending intents across metadata/loading publications.
        // A changed layout can retain exact toggles; position/range gestures must
        // not select newly inserted or previously hidden rows.
        p.tree.preview.retain(|(revision, action)| {
            *revision > publication.selection_revision
                && (same_layout
                    || matches!(action, TreeSelectionAction::Toggle(id)
                if view.index.row_positions.contains_key(id)))
        });
        p.tree.replay = p.tree.preview.iter().cloned().collect();
        p.tree.revision = p.tree.revision.max(publication.selection_revision);
        p.tree.applied = publication.selection_revision;
        // Requests against the retired Arc cannot change this publication.
        // Surviving intents remain fenced by preview until their replay settles.
        p.tree.resolved = p.tree.revision;
        p.tree.commit_pending = true;
        p.tree.navigation = None;
        self.retired.push(Box::new(p.tree.pending.take()));
        self.retired
            .push(Box::new(p.tree.publication.replace(publication)));
        true
    }
}
