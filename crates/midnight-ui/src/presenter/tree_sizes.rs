//! Directory sizes retain occurrence identities across projection changes.
use super::*;
use std::collections::VecDeque;

const MAX_PENDING: usize = 8;

struct Summary {
    view: Arc<TreeProjection>,
    selection: Arc<TreeSelectionSnapshot>,
    revision: SelectionRevision,
    version: u64,
    value: SelectedFolderSizes,
}

struct Pending {
    scope: TreeSizeScope,
    selection: Option<(TreeViewStamp, SelectionRevision)>,
}

#[derive(Default)]
pub(super) struct State {
    snapshot: Option<Arc<TreeSizeSnapshot>>,
    full: Option<(TreeViewStamp, TreeRequestId)>,
    entries: Vec<(TreeRowId, TreeRequestId)>,
    pending: VecDeque<Pending>,
    selected: Option<Summary>,
}

impl State {
    pub(super) fn diagnostic_revision(&self) -> u64 {
        self.snapshot
            .as_ref()
            .map_or(0, |snapshot| snapshot.version)
    }

    pub(super) fn size(&self, row: TreeRowId) -> Option<&FolderSize> {
        self.snapshot.as_ref()?.get(row)
    }

    pub(super) fn running(&self, row: TreeRowId) -> bool {
        self.entries.iter().any(|(id, _)| *id == row)
            || self
                .snapshot
                .as_ref()
                .is_some_and(|sizes| sizes.running(row))
    }
}

impl Pane {
    pub(super) fn tree_size_summary(&self) -> Option<SelectedFolderSizes> {
        let publication = self.tree.publication.as_ref()?;
        let summary = self.tree.sizes.selected.as_ref()?;
        (self.tree.preview.is_empty()
            && Arc::ptr_eq(&summary.view, &publication.view)
            && Arc::ptr_eq(&summary.selection, &publication.selection)
            && summary.revision == publication.selection_revision)
            .then_some(summary.value)
    }
}

impl Presenter {
    pub(super) fn tree_folder_sizes(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        let p = &mut self.panes[pane.index()];
        if let Some((stamp, request)) = p.tree.sizes.full {
            if !send(Command::Tree(Box::new(TreeCommand::Sizes(
                TreeSizeCommand::Cancel { stamp, request },
            )))) {
                p.set_error(
                    "Folder size queue is busy. Try cancelling again.",
                    &mut self.retired,
                );
            }
            return;
        }
        if p.loading || p.tree.sizes.pending.len() >= MAX_PENDING {
            return;
        }
        if !p
            .tree
            .sizes
            .pending
            .iter()
            .any(|pending| matches!(pending.scope, TreeSizeScope::SelectionOrVisible))
        {
            let Some(view) = p.tree_view() else { return };
            let revision = if p.tree.preview.is_empty() {
                p.tree.applied
            } else {
                p.tree.revision
            };
            let selection = Some((view.stamp, revision));
            p.tree.sizes.pending.push_back(Pending {
                scope: TreeSizeScope::SelectionOrVisible,
                selection,
            });
        }
        self.flush_tree_sizes(pane, send);
    }

    pub(super) fn measure_tree_row(
        &mut self,
        pane: PaneId,
        position: usize,
        send: &dyn Fn(Command) -> bool,
    ) {
        let p = &mut self.panes[pane.index()];
        if p.loading || !p.tree_input_ready() {
            return;
        }
        let Some(row) = position
            .checked_sub(p.parent_offset())
            .and_then(|row| p.tree_row(row))
        else {
            return;
        };
        if row.entry.kind != EntryKind::Directory || !row.selectable || row.context_only {
            return;
        }
        let id = row.id;
        if p.tree.sizes.full.is_some()
            || p.tree.sizes.entries.iter().any(|(row, _)| *row == id)
            || p.tree
                .sizes
                .pending
                .iter()
                .any(|pending| matches!(pending.scope, TreeSizeScope::Single(row) if row == id))
            || p.tree.sizes.pending.len() + p.tree.sizes.entries.len() >= MAX_PENDING
        {
            return;
        }
        p.tree.sizes.pending.push_back(Pending {
            scope: TreeSizeScope::Single(id),
            selection: None,
        });
        self.flush_tree_sizes(pane, send);
    }

    pub(super) fn flush_tree_size_acks(&mut self, send: &dyn Fn(Command) -> bool) {
        for pane in [PaneId::Left, PaneId::Right] {
            if let Some((stamp, version)) = self.tree_size_acks[pane.index()]
                && send(Command::Tree(Box::new(TreeCommand::Sizes(
                    TreeSizeCommand::Acknowledge { stamp, version },
                ))))
            {
                self.tree_size_acks[pane.index()] = None;
            }
        }
    }

    pub(super) fn flush_tree_sizes(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        let p = &mut self.panes[pane.index()];
        if p.loading || !p.tree_input_ready() || !p.tree.preview.is_empty() {
            return;
        }
        let Some(publication) = &p.tree.publication else {
            return;
        };
        while let Some(pending) = p.tree.sizes.pending.front() {
            if pending.selection.is_some_and(|(stamp, revision)| {
                stamp != publication.view.stamp || revision != publication.selection_revision
            }) {
                p.tree.sizes.pending.pop_front();
                self.desktop.status =
                    "Folder size request cancelled because the view or selection changed.".into();
                continue;
            }
            if let TreeSizeScope::Single(row) = pending.scope
                && !publication.view.index.row_positions.contains_key(&row)
            {
                p.tree.sizes.pending.pop_front();
                continue;
            }
            let Some(request) = p.tree.request.checked_next() else {
                return;
            };
            let command = TreeSizeRequest {
                view: publication.view.clone(),
                request,
                selection_revision: publication.selection_revision,
                selection: publication.selection.clone(),
                scope: pending.scope,
            };
            if !send(Command::Tree(Box::new(TreeCommand::Sizes(
                TreeSizeCommand::Compute(command),
            )))) {
                p.set_error("Folder size queue is busy. Try again.", &mut self.retired);
                return;
            }
            p.tree.request = request;
            match p
                .tree
                .sizes
                .pending
                .pop_front()
                .map(|pending| pending.scope)
            {
                Some(TreeSizeScope::SelectionOrVisible) => {
                    p.tree.sizes.full = Some((publication.view.stamp, request));
                }
                Some(TreeSizeScope::Single(row)) => p.tree.sizes.entries.push((row, request)),
                None => {}
            }
        }
    }

    pub(super) fn apply_tree_sizes(
        &mut self,
        event: TreeSizeEvent,
        send: &dyn Fn(Command) -> bool,
    ) {
        match event {
            TreeSizeEvent::Snapshot { sizes } => {
                let pane = sizes.stamp.pane;
                self.tree_size_acks[pane.index()] = Some((sizes.stamp, sizes.version));
                self.flush_tree_size_acks(send);
                let p = &mut self.panes[pane.index()];
                if p.tree_view()
                    .is_some_and(|view| same_session(view.stamp, sizes.stamp))
                    && p.generation == sizes.stamp.navigation.0
                    && p.tree
                        .sizes
                        .snapshot
                        .as_ref()
                        .is_none_or(|previous| sizes.version > previous.version)
                {
                    self.retired
                        .push(Box::new(p.tree.sizes.snapshot.replace(sizes)));
                } else {
                    self.retired.push(Box::new(sizes));
                }
            }
            TreeSizeEvent::Selected {
                view,
                selection_revision,
                selection,
                version,
                summary,
            } => {
                let p = &mut self.panes[view.stamp.pane.index()];
                let current = p.tree.publication.as_ref().is_some_and(|publication| {
                    Arc::ptr_eq(&view, &publication.view)
                        && Arc::ptr_eq(&selection, &publication.selection)
                        && selection_revision == publication.selection_revision
                });
                let newer = p.tree.sizes.selected.as_ref().is_none_or(|previous| {
                    !Arc::ptr_eq(&previous.view, &view)
                        || !Arc::ptr_eq(&previous.selection, &selection)
                        || previous.revision != selection_revision
                        || version > previous.version
                });
                if current && newer && p.generation == view.stamp.navigation.0 {
                    self.retired
                        .push(Box::new(p.tree.sizes.selected.replace(Summary {
                            view,
                            selection,
                            revision: selection_revision,
                            version,
                            value: summary,
                        })));
                } else {
                    self.retired.push(Box::new((view, selection)));
                }
            }
            TreeSizeEvent::Finished { stamp, request, .. } => {
                self.finish_tree_size(stamp, request, None)
            }
            TreeSizeEvent::Rejected {
                stamp,
                request,
                error,
            } => {
                self.finish_tree_size(stamp, request, Some(error.as_str()));
            }
        }
    }

    fn finish_tree_size(
        &mut self,
        stamp: TreeViewStamp,
        request: TreeRequestId,
        error: Option<&str>,
    ) {
        let p = &mut self.panes[stamp.pane.index()];
        if !p
            .tree_view()
            .is_some_and(|view| same_session(view.stamp, stamp))
        {
            return;
        }
        let full = p.tree.sizes.full.is_some_and(|(_, id)| id == request);
        let entry = p.tree.sizes.entries.iter().any(|(_, id)| *id == request);
        if full {
            p.tree.sizes.full = None;
        }
        p.tree.sizes.entries.retain(|(_, id)| *id != request);
        if (full || entry)
            && let Some(error) = error
        {
            p.set_error(error, &mut self.retired);
        }
    }
}

fn same_session(left: TreeViewStamp, right: TreeViewStamp) -> bool {
    left.pane == right.pane && left.navigation == right.navigation && left.session == right.session
}
