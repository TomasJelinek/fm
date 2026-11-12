//! Completed roots become small ordered selection edits, never filesystem work on UI.
use super::*;

#[derive(Clone, PartialEq, Eq)]
struct Origin {
    context: NavigationId,
    path: PathBuf,
    archive: Option<(u64, u64)>,
    tree: Option<(NavigationId, TreeSessionId)>,
}
impl Origin {
    fn of(p: &Pane) -> Self {
        Self {
            context: p.selection_context,
            path: p.display_path().to_owned(),
            archive: p.archive.as_ref().and_then(|a| {
                a.location
                    .mount_id()
                    .map(|mount| (mount, a.location.directory))
            }),
            tree: p
                .tree_view()
                .map(|view| (view.stamp.navigation, view.stamp.session)),
        }
    }
}

/// Freeze the panel contexts at the gesture, before delayed drag admission.
pub(super) struct CapturedOrigins([Origin; 2]);

#[derive(Clone, PartialEq, Eq, Hash)]
enum Item {
    Native(PathBuf),
    Archive(EntryId),
    Tree(TreeRowId),
}
struct Sent {
    navigation: u64,
    revision: SelectionRevision,
    items: HashSet<Item>,
    tree: Option<TreeViewStamp>,
}
struct Pending {
    origin: Origin,
    items: HashSet<Item>,
    sent: Option<Sent>,
}
#[derive(Default)]
pub(super) struct CopySelections {
    origins: HashMap<OperationId, [Origin; 2]>,
    pending: [Option<Pending>; 2],
}
impl CopySelections {
    pub(super) fn pending_for(&self, pane: PaneId) -> bool {
        self.pending[pane.index()]
            .as_ref()
            .is_some_and(|pending| pending.sent.is_some() || !pending.items.is_empty())
    }

    pub(super) fn retain_live(&mut self, progress: &[Arc<CopyProgress>]) {
        self.origins.retain(|id, _| {
            progress
                .iter()
                .any(|job| job.id == *id && !job.state.is_terminal())
        });
    }
}

impl Presenter {
    pub(super) fn capture_copy_origins(&self) -> CapturedOrigins {
        CapturedOrigins(self.panes.each_ref().map(Origin::of))
    }

    pub(super) fn track_captured_copy_selection(
        &mut self,
        id: OperationId,
        origins: CapturedOrigins,
    ) {
        self.copies.selection.origins.insert(id, origins.0);
    }

    pub(super) fn track_copy_selection(&mut self, id: OperationId) {
        self.track_captured_copy_selection(id, self.capture_copy_origins());
    }

    pub(super) fn complete_copy_selection(&mut self, id: OperationId, source: CompletedCopySource) {
        let Some(origins) = self.copies.selection.origins.get(&id) else {
            return;
        };
        for pane in [PaneId::Left, PaneId::Right] {
            let slot = pane.index();
            let p = &self.panes[slot];
            let origin = Origin::of(p);
            if origin != origins[slot] {
                continue;
            }
            let item = match &source {
                CompletedCopySource::Tree(occurrence)
                    if occurrence.stamp.pane == pane
                        && origin.tree
                            == Some((occurrence.stamp.navigation, occurrence.stamp.session))
                        && p.tree_view().is_some_and(|view| {
                            view.index.row_positions.contains_key(&occurrence.row)
                        }) =>
                {
                    Item::Tree(occurrence.row)
                }
                CompletedCopySource::Native(path)
                    if p.archive.is_none() && path.parent() == Some(origin.path.as_path()) =>
                {
                    Item::Native(path.clone())
                }
                CompletedCopySource::Archive(id)
                    if origin.archive.is_some_and(|(mount, _)| mount == id.mount)
                        && p.entry_by_id(EntryId(id.entry))
                            .and_then(|e| e.virtual_node.as_ref())
                            .is_some_and(|node| node.id == *id) =>
                {
                    Item::Archive(EntryId(id.entry))
                }
                _ => continue,
            };
            let pending = &mut self.copies.selection.pending[slot];
            if pending
                .as_ref()
                .is_some_and(|pending| pending.origin != origin)
            {
                self.retired.push(Box::new(pending.take()));
            }
            let pending = pending.get_or_insert_with(|| Pending {
                origin,
                items: HashSet::new(),
                sent: None,
            });
            pending.items.insert(item);
        }
    }

    pub(super) fn forget_copy_selection(&mut self, id: OperationId) {
        self.copies.selection.origins.remove(&id);
    }

    pub(super) fn copy_selection_result(
        &mut self,
        pane: PaneId,
        navigation: NavigationId,
        revision: SelectionRevision,
        success: bool,
    ) {
        let Some(pending) = &mut self.copies.selection.pending[pane.index()] else {
            return;
        };
        if pending.sent.as_ref().is_some_and(|sent| {
            sent.navigation == navigation.0
                && (sent.revision == revision || (!success && sent.revision < revision))
        }) {
            // The backend coalesces queue refusals to the newest revision. A
            // later refusal can therefore be the only acknowledgement of ours.
            let sent = pending.sent.take().expect("matched completion revision");
            if !success {
                pending.items.extend(sent.items);
            }
        }
    }

    pub(super) fn tree_copy_selection_result(
        &mut self,
        stamp: TreeViewStamp,
        revision: SelectionRevision,
        success: bool,
    ) {
        if self.copies.selection.pending[stamp.pane.index()]
            .as_ref()
            .is_some_and(|pending| {
                pending
                    .sent
                    .as_ref()
                    .is_some_and(|sent| sent.tree == Some(stamp))
            })
        {
            self.copy_selection_result(stamp.pane, stamp.navigation, revision, success);
        }
    }

    pub(super) fn flush_copy_selection(&mut self, send: &dyn Fn(Command) -> bool) {
        for pane in [PaneId::Left, PaneId::Right] {
            let slot = pane.index();
            let Some(mut pending) = self.copies.selection.pending[slot].take() else {
                continue;
            };
            let p = &self.panes[slot];
            if pending.origin != Origin::of(p) {
                self.retired.push(Box::new(pending));
                continue;
            }
            // Same-folder refresh resets selection revisions. Retry the exact paths
            // against its new snapshot; never reuse numeric native row identities.
            if pending.sent.as_ref().is_some_and(|sent| {
                sent.navigation != p.generation || sent.tree != p.tree_view().map(|view| view.stamp)
            }) {
                pending
                    .items
                    .extend(pending.sent.take().expect("changed navigation").items);
            }
            if pending.sent.is_some()
                || p.loading
                || p.search_view_waiting()
                || (p.tree_view().is_some()
                    && (!p.tree_input_ready() || !p.tree.preview.is_empty()))
            {
                self.copies.selection.pending[slot] = Some(pending);
                continue;
            }
            // Bound each edit and its immediate row feedback independently of folder size.
            let mut batch: HashSet<_> = pending.items.iter().take(64).cloned().collect();
            if let Some(view) = p.tree_view() {
                batch.retain(|item| match item {
                    Item::Tree(row) if !view.index.row_positions.contains_key(row) => {
                        // A collapsed/filtered occurrence is already absent from selection.
                        pending.items.remove(item);
                        false
                    }
                    _ => true,
                });
            }
            if batch.is_empty() {
                if !pending.items.is_empty() {
                    self.copies.selection.pending[slot] = Some(pending);
                }
                continue;
            }
            let accepted = if p.tree_view().is_some() {
                let rows = batch
                    .iter()
                    .filter_map(|item| match item {
                        Item::Tree(row) => Some(*row),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                self.send_tree_selection(pane, TreeSelectionAction::Remove(rows.into()), send)
            } else if p.archive.is_some() {
                let ids = batch
                    .iter()
                    .filter_map(|item| match item {
                        Item::Archive(id) => Some(*id),
                        _ => None,
                    })
                    .collect();
                self.archive_bulk_selection(
                    pane,
                    ArchiveSelectionAction::Remove(Arc::new(ids)),
                    send,
                )
            } else {
                let paths = batch
                    .iter()
                    .filter_map(|item| match item {
                        Item::Native(path) => Some(path.clone()),
                        _ => None,
                    })
                    .collect();
                self.send_selection(pane, SelectionAction::Remove(Arc::new(paths)), send)
            };
            if accepted {
                for item in &batch {
                    pending.items.remove(item);
                }
                let p = &self.panes[slot];
                pending.sent = Some(Sent {
                    navigation: p.generation,
                    revision: if p.tree_view().is_some() {
                        p.tree.revision
                    } else {
                        p.archive
                            .as_ref()
                            .map_or(p.selection_revision, |a| a.selection_revision)
                    },
                    items: batch,
                    tree: p.tree_view().map(|view| view.stamp),
                });
            }
            self.copies.selection.pending[slot] = Some(pending);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locally_cancelled_admitted_drop_releases_completion_origin() {
        let root = std::env::temp_dir();
        let mut p = Presenter::new(root.clone(), root.clone());
        p.apply_with_sender(
            Event::DragDrop(DragDropEvent::DropAccepted {
                drag: None,
                intent: TransferIntent::Copy,
                verification: None,
                token: DropToken(1),
                pane: PaneId::Right,
                item_count: 1,
                destination: Arc::new(root),
            }),
            &|_| false,
        );
        let id = p.copies.progress[0].id;
        assert!(p.copies.selection.origins.contains_key(&id));
        assert!(p.cancel_pending_drop(id, &|_| true));
        assert!(!p.copies.selection.origins.contains_key(&id));
    }
}
