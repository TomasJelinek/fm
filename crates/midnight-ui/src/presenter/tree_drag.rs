//! Exact tree occurrences for drag sources and destinations. Provider resolution
//! and final destruction stay on backend workers, including rejected receipts.
use super::*;

#[derive(Default)]
pub(super) struct State {
    pub source: Option<Source>,
    pub destination: Option<Destination>,
}

pub(super) struct Source {
    id: DragId,
    row: TreeRowId,
    capture: tree_capture::Capture,
    files: Option<Arc<dyn CapturedClipboardFiles>>,
    retire_requested: bool,
}

pub(super) struct Destination {
    id: DragId,
    view: Arc<TreeProjection>,
    row: TreeRowId,
    request: TreeRequestId,
    target: Option<Arc<dyn CapturedDropDirectory>>,
    retire_requested: bool,
}

impl Presenter {
    pub(super) fn begin_tree_drag(
        &mut self,
        pane: PaneId,
        row: usize,
        identity: &str,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        if !self.owned_command_scope_available() {
            return false;
        }
        let p = &self.panes[pane.index()];
        // The dragged row is validated below; the focused row does not matter.
        if !p.native_capabilities() || !p.tree_input_ready() {
            return false;
        }
        let Some(display) = row
            .checked_sub(p.parent_offset())
            .and_then(|position| p.tree_row(position))
            .filter(|r| r.selectable && !r.context_only && r.entry.virtual_node.is_none())
            .filter(|r| identity == p.row_identity(&r.entry))
        else {
            return false;
        };
        let dragged = display.id;
        let Some(id) = self.start_file_drag(pane, row) else {
            return false;
        };
        let Some(capture) = tree_capture::Capture::new(&self.panes[pane.index()]) else {
            self.cancel_file_drag(send);
            return false;
        };
        self.drag_drop.tree.source = Some(Source {
            id,
            row: dragged,
            capture,
            files: None,
            retire_requested: false,
        });
        self.flush_tree_drag(send);
        self.drag_drop.lifecycle.active().is_some()
    }

    pub(super) fn cancel_tree_drag(&mut self, pane: Option<PaneId>) {
        let source = self
            .drag_drop
            .tree
            .source
            .as_mut()
            .filter(|source| pane.is_none_or(|pane| pane == source.capture.view.stamp.pane))
            .map(|source| {
                source.capture.cancel.cancel();
                source.retire_requested = true;
                source.id
            });
        if let Some(id) = source {
            self.queue_cancelled_drag(id);
        }
        let destination = self
            .drag_drop
            .tree
            .destination
            .as_mut()
            .filter(|destination| pane.is_none_or(|pane| pane == destination.view.stamp.pane))
            .map(|destination| {
                destination.retire_requested = true;
                destination.id
            });
        if let Some(id) = destination {
            self.queue_cancelled_drag(id);
        }
        if self.owned_command_scope_available() {
            self.flush_deferred_tree_drag_owners();
        }
    }

    pub(super) fn finish_tree_drag(&mut self, id: DragId) {
        if let Some(source) = self
            .drag_drop
            .tree
            .source
            .as_mut()
            .filter(|source| source.id == id)
        {
            source.capture.cancel.cancel();
            source.retire_requested = true;
        }
        if let Some(destination) = self
            .drag_drop
            .tree
            .destination
            .as_mut()
            .filter(|target| target.id == id)
        {
            destination.retire_requested = true;
        }
        if self.owned_command_scope_available() {
            self.flush_deferred_tree_drag_owners();
        }
    }

    pub(super) fn flush_deferred_tree_drag_owners(&mut self) {
        if self
            .drag_drop
            .tree
            .source
            .as_ref()
            .is_some_and(|source| source.retire_requested)
        {
            self.retired
                .push(Box::new(self.drag_drop.tree.source.take()));
        }
        if self
            .drag_drop
            .tree
            .destination
            .as_ref()
            .is_some_and(|destination| destination.retire_requested)
        {
            self.retired
                .push(Box::new(self.drag_drop.tree.destination.take()));
        }
    }

    pub(super) fn matches_tree_drag_capture(
        &self,
        stamp: TreeViewStamp,
        request: TreeRequestId,
    ) -> bool {
        self.drag_drop.tree.source.as_ref().is_some_and(|source| {
            !source.retire_requested
                && source.capture.view.stamp == stamp
                && source.capture.request == Some(request)
        })
    }

    pub(super) fn tree_drag_captured(
        &mut self,
        result: Result<Arc<dyn CapturedClipboardFiles>, TreeDiagnostic>,
        send: &dyn Fn(Command) -> bool,
    ) {
        debug_assert!(self.owned_command_scope_available());
        match result {
            Ok(files) if files.intent() == FileClipboardIntent::Copy => {
                let source = self.drag_drop.tree.source.as_mut().expect("matched drag");
                if source.files.is_none() {
                    source.files = Some(files);
                } else {
                    self.retired.push(Box::new(files));
                }
            }
            result => {
                self.desktop.status = match &result {
                    Err(error) => format!("Drag unavailable: {}", error.as_str()),
                    Ok(_) => "Drag unavailable: captured source intent changed.".into(),
                };
                self.retired.push(Box::new(result));
                self.cancel_file_drag(send);
                return;
            }
        }
        self.flush_tree_drag(send);
    }

    pub(super) fn flush_tree_drag(&mut self, send: &dyn Fn(Command) -> bool) {
        if !self.owned_command_scope_available() {
            return;
        }
        self.flush_deferred_tree_drag_owners();
        if let Some(mut source) = self.drag_drop.tree.source.take() {
            let pane = source.capture.view.stamp.pane;
            let live = self.drag_drop.pointer_down
                && self.drag_drop.lifecycle.active().is_some_and(|gesture| {
                    gesture.id == source.id.0 && gesture.source == self.drag_stamp(pane)
                });
            let status = source
                .capture
                .status(&self.panes[pane.index()], self.operations.closing || !live);
            match status {
                tree_capture::Status::Gone | tree_capture::Status::SelectionChanged => {
                    source.capture.cancel.cancel();
                    self.queue_cancelled_drag(source.id);
                    self.retired.push(Box::new(source));
                    self.cancel_file_drag(send);
                    return;
                }
                tree_capture::Status::Ready(request, publication) => {
                    if send(Command::Tree(Box::new(TreeCommand::Clipboard(
                        TreeClipboardRequest {
                            scope: TreeClipboardScope::Dragged(source.row),
                            intent: FileClipboardIntent::Copy,
                            view: source.capture.view.clone(),
                            request,
                            selection_revision: publication.selection_revision,
                            selection: publication.selection,
                            fallback: Some(source.row),
                            cancel: source.capture.cancel.clone(),
                        },
                    )))) {
                        self.panes[pane.index()].tree.request = request;
                        source.capture.request = Some(request);
                    }
                }
                tree_capture::Status::Waiting => {}
            }
            if source.files.as_ref().is_some_and(|files| {
                send(Command::PrepareCapturedDrag {
                    id: source.id,
                    pane,
                    navigation: source.capture.view.stamp.navigation,
                    source: files.clone(),
                })
            }) {
                // The backend now owns admission. Cancelling this capture here
                // would revoke a successful drag before the user releases it.
                self.retired.push(Box::new(source));
            } else {
                self.drag_drop.tree.source = Some(source);
            }
        }
        self.flush_tree_drop(send);
    }

    pub(super) fn prepare_internal_drag_target(
        &mut self,
        target: InternalDropDestination,
        id: DragId,
    ) -> Option<InternalDropDestination> {
        if !self.owned_command_scope_available() {
            return None;
        }
        let InternalDropDestination::TreeOccurrence { view, row, .. } = target else {
            let (pane, navigation, entry) = match &target {
                InternalDropDestination::PaneRoot { pane, navigation } => {
                    (*pane, *navigation, None)
                }
                InternalDropDestination::PanelEntry {
                    pane,
                    navigation,
                    entry,
                } => (*pane, *navigation, Some(*entry)),
                InternalDropDestination::TreeOccurrence { .. } => unreachable!(),
            };
            let p = &self.panes[pane.index()];
            let valid = !p.loading
                && p.native_capabilities()
                && p.successful_generation == Some(navigation)
                && navigation == NavigationId(p.generation)
                && entry.is_none_or(|id| {
                    p.tree_view().is_none()
                        && p.display_index()
                            .by_id
                            .get(&id)
                            .and_then(|position| p.display_entry(*position))
                            .is_some_and(|entry| {
                                entry.kind == EntryKind::Directory && entry.virtual_node.is_none()
                            })
                });
            return valid.then_some(target);
        };
        let p = &mut self.panes[view.stamp.pane.index()];
        let request = p.tree.request.0.checked_add(1).map(TreeRequestId);
        if self.drag_drop.tree.destination.is_some()
            || p.loading
            || !p.tree_input_ready()
            || !p.native_capabilities()
            || !view
                .index
                .row_positions
                .get(&row)
                .and_then(|position| view.rows.get(*position))
                .is_some_and(|row| {
                    row.selectable
                        && row.navigable
                        && !row.context_only
                        && row.entry.kind == EntryKind::Directory
                        && row.entry.virtual_node.is_none()
                })
            || !p
                .tree_view()
                .is_some_and(|current| Arc::ptr_eq(current, &view))
            || request.is_none()
        {
            self.retired.push(Box::new(view));
            return None;
        }
        let request = request.unwrap();
        p.tree.request = request;
        self.drag_drop.tree.destination = Some(Destination {
            id,
            view: view.clone(),
            row,
            request,
            target: None,
            retire_requested: false,
        });
        Some(InternalDropDestination::TreeOccurrence { view, row, request })
    }

    pub(super) fn tree_drop_captured(
        &mut self,
        stamp: TreeViewStamp,
        request: TreeRequestId,
        id: DragId,
        result: Result<Arc<dyn CapturedDropDirectory>, TreeDiagnostic>,
        send: &dyn Fn(Command) -> bool,
    ) {
        debug_assert!(self.owned_command_scope_available());
        let mut result = Some(result);
        if self.tree_drag_preview_captured(stamp, request, id, &mut result) {
            return;
        }
        let result = result.expect("unhandled final drop receipt");
        let Some(pending) = self.drag_drop.tree.destination.as_mut().filter(|pending| {
            !pending.retire_requested
                && pending.id == id
                && pending.view.stamp == stamp
                && pending.request == request
        }) else {
            self.retired.push(Box::new(result));
            return;
        };
        match result {
            Ok(target) if target.stamp() == stamp && target.row() == pending.row => {
                if pending.target.is_none() {
                    pending.target = Some(target);
                } else {
                    self.retired.push(Box::new(target));
                }
            }
            result => {
                self.desktop.status = match &result {
                    Err(error) => format!("Drop unavailable: {}", error.as_str()),
                    Ok(_) => "Drop unavailable: captured folder changed.".into(),
                };
                self.retired.push(Box::new(result));
                self.cancel_tree_drag(None);
            }
        }
        self.flush_tree_drop(send);
    }

    fn flush_tree_drop(&mut self, send: &dyn Fn(Command) -> bool) {
        if !self.owned_command_scope_available() {
            return;
        }
        self.flush_deferred_tree_drag_owners();
        let Some(pending) = self.drag_drop.tree.destination.take() else {
            return;
        };
        let p = &self.panes[pending.view.stamp.pane.index()];
        if self.operations.closing
            || p.loading
            || !p.tree_input_ready()
            || !p
                .tree_view()
                .is_some_and(|view| Arc::ptr_eq(view, &pending.view))
        {
            self.queue_cancelled_drag(pending.id);
            self.retired.push(Box::new(pending));
        } else if pending.target.as_ref().is_some_and(|target| {
            send(Command::CompleteInternalDrop {
                id: pending.id,
                target: target.clone(),
            })
        }) {
            self.retired.push(Box::new(pending));
        } else {
            self.drag_drop.tree.destination = Some(pending);
        }
    }
}

#[cfg(test)]
mod ownership_tests {
    use super::*;
    use std::{cell::RefCell, sync::Mutex};

    fn presenter() -> Presenter {
        let root = std::env::temp_dir();
        let mut p = Presenter::new(root.clone(), root);
        let pane = &mut p.panes[0];
        pane.loading = false;
        pane.successful_generation = Some(NavigationId(pane.generation));
        let root = TreeRootSnapshot {
            pane: PaneId::Left,
            navigation: NavigationId(pane.generation),
            entries: pane.entries.clone(),
            index: pane.index.clone(),
        };
        let stamp = TreeViewStamp {
            pane: root.pane,
            navigation: root.navigation,
            session: TreeSessionId(1),
            revision: TreeRevision(1),
        };
        let view =
            Arc::new(TreeProjection::new(root, stamp, TreeLayoutRevision(1), vec![]).unwrap());
        let selection = Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![]).unwrap());
        pane.tree.publication =
            Some(TreePublication::new(view, SelectionRevision(0), selection).unwrap());
        p
    }

    #[derive(Debug, Default)]
    struct DropThreads(Mutex<Vec<std::thread::ThreadId>>);

    impl DropThreads {
        fn assert_live(&self) {
            assert!(
                self.0.lock().unwrap().is_empty(),
                "native owner dropped before handoff"
            );
        }

        fn assert_worker_drop(&self) {
            let threads = self.0.lock().unwrap();
            assert_eq!(threads.len(), 1);
            assert_ne!(threads[0], std::thread::current().id());
        }
    }

    #[derive(Debug)]
    struct Files(Arc<DropThreads>);

    impl CapturedClipboardFiles for Files {
        fn intent(&self) -> FileClipboardIntent {
            FileClipboardIntent::Copy
        }

        fn resolve(&self, _: &Cancellation) -> Result<FileClipboard, FsError> {
            panic!("UI must not resolve a captured source")
        }
    }

    impl Drop for Files {
        fn drop(&mut self) {
            self.0.0.lock().unwrap().push(std::thread::current().id());
        }
    }

    #[derive(Debug)]
    struct Folder {
        stamp: TreeViewStamp,
        drops: Arc<DropThreads>,
    }

    impl CapturedDropDirectory for Folder {
        fn stamp(&self) -> TreeViewStamp {
            self.stamp
        }

        fn row(&self) -> TreeRowId {
            TreeRowId(1)
        }

        fn resolve(&self, _: &Cancellation) -> Result<PathBuf, FsError> {
            panic!("UI must not resolve a captured destination")
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            self.drops
                .0
                .lock()
                .unwrap()
                .push(std::thread::current().id());
        }
    }

    #[test]
    fn unclaimed_drag_owners_remain_in_place_while_cancel_is_live() {
        let mut p = presenter();
        let view = p.panes[0].tree_view().unwrap().clone();
        let source = p.drag_stamp(PaneId::Left);
        let id = DragId(p.drag_drop.lifecycle.begin(source).unwrap());
        p.drag_drop.pointer_down = true;
        let source_drops = Arc::new(DropThreads::default());
        let target_drops = Arc::new(DropThreads::default());
        let mut capture = tree_capture::Capture::optional_targets(&p.panes[0]).unwrap();
        capture.request = Some(TreeRequestId(1));
        p.drag_drop.tree.source = Some(Source {
            id,
            row: TreeRowId(1),
            capture,
            files: Some(Arc::new(Files(source_drops.clone()))),
            retire_requested: false,
        });
        p.drag_drop.tree.destination = Some(Destination {
            id,
            view: view.clone(),
            row: TreeRowId(1),
            request: TreeRequestId(2),
            target: Some(Arc::new(Folder {
                stamp: view.stamp,
                drops: target_drops.clone(),
            })),
            retire_requested: false,
        });
        p.require_owned_command_scope();
        for _ in 0..8 {
            p.flush_drag_commands(&|_| panic!("blocked callback forwarded an owner"));
        }
        assert_eq!(
            Arc::strong_count(
                p.drag_drop
                    .tree
                    .source
                    .as_ref()
                    .unwrap()
                    .files
                    .as_ref()
                    .unwrap()
            ),
            1
        );
        assert_eq!(
            Arc::strong_count(
                p.drag_drop
                    .tree
                    .destination
                    .as_ref()
                    .unwrap()
                    .target
                    .as_ref()
                    .unwrap()
            ),
            1
        );
        let controls = RefCell::new(Vec::new());
        p.cancel_file_drag(&|command| {
            controls.borrow_mut().push(command);
            true
        });
        assert!(
            matches!(controls.borrow().as_slice(), [Command::CancelDrag(actual)] if *actual == id)
        );
        for _ in 0..8 {
            p.cancel_tree_drag(None);
            p.flush_deferred_drag_owners();
        }
        assert!(p.drag_drop.tree.source.is_some());
        assert!(p.drag_drop.tree.destination.is_some());
        assert!(p.take_retired().is_empty());
        source_drops.assert_live();
        target_drops.assert_live();

        p.set_owned_command_scope_active(true);
        p.flush_deferred_drag_owners();
        assert!(p.drag_drop.tree.source.is_none());
        assert!(p.drag_drop.tree.destination.is_none());
        source_drops.assert_live();
        target_drops.assert_live();
        let retired = p.take_retired();
        std::thread::spawn(move || drop(retired)).join().unwrap();
        source_drops.assert_worker_drop();
        target_drops.assert_worker_drop();
    }

    #[test]
    fn unclaimed_cancelled_clipboard_source_stays_in_existing_slot() {
        let mut p = presenter();
        let capture = tree_capture::Capture::optional_targets(&p.panes[0]).unwrap();
        p.start_tree_action(
            capture,
            FileClipboardIntent::Copy,
            TreeClipboardScope::SelectionOrFallback,
            tree_clipboard::Purpose::Clipboard,
            &|_| true,
        );
        let pending = p.file_actions.tree_clipboard.as_ref().unwrap();
        let (stamp, request) = (pending.capture.view.stamp, pending.capture.request.unwrap());
        let drops = Arc::new(DropThreads::default());
        p.tree_clipboard_captured(stamp, request, Ok(Arc::new(Files(drops.clone()))), &|_| {
            false
        });
        p.require_owned_command_scope();
        for _ in 0..8 {
            p.flush_tree_clipboard(&|_| panic!("blocked clipboard source was cloned"));
            p.cancel_tree_clipboard(None);
            p.flush_deferred_drag_owners();
        }
        let pending = p.file_actions.tree_clipboard.as_ref().unwrap();
        assert!(pending.capture.cancel.is_cancelled());
        assert_eq!(Arc::strong_count(pending.source.as_ref().unwrap()), 1);
        assert!(p.take_retired().is_empty());
        drops.assert_live();

        p.set_owned_command_scope_active(true);
        p.flush_deferred_drag_owners();
        assert!(p.file_actions.tree_clipboard.is_none());
        drops.assert_live();
        let retired = p.take_retired();
        std::thread::spawn(move || drop(retired)).join().unwrap();
        drops.assert_worker_drop();
    }

    #[test]
    fn unclaimed_cancelled_preview_target_stays_in_existing_slot() {
        let mut p = presenter();
        let view = p.panes[0].tree_view().unwrap().clone();
        let source = p.drag_stamp(PaneId::Left);
        let id = DragId(p.drag_drop.lifecycle.begin(source).unwrap());
        assert!(p.drag_drop.lifecycle.prepared(id.0));
        p.drag_drop.pointer_down = true;
        p.update_drag_preview(
            Some(InternalDropDestination::TreeOccurrence {
                view: view.clone(),
                row: TreeRowId(1),
                request: TreeRequestId(0),
            }),
            InternalDropIntent::Copy,
            &|_| true,
        );
        let drops = Arc::new(DropThreads::default());
        let mut result = Some(Ok(Arc::new(Folder {
            stamp: view.stamp,
            drops: drops.clone(),
        }) as Arc<dyn CapturedDropDirectory>));
        let request = p.panes[0].tree.request;
        assert!(p.tree_drag_preview_captured(view.stamp, request, id, &mut result));
        assert!(result.is_none());
        p.require_owned_command_scope();
        for _ in 0..8 {
            p.update_drag_preview(None, InternalDropIntent::Copy, &|_| {
                panic!("unclaimed preview forwarded a captured target")
            });
            p.cancel_drag_preview();
            p.flush_deferred_drag_owners();
        }
        assert_eq!(
            p.drag_preview_status(),
            drag_drop::preview::Status::Rejected
        );
        assert!(p.take_retired().is_empty());
        drops.assert_live();

        p.set_owned_command_scope_active(true);
        p.flush_deferred_drag_owners();
        drops.assert_live();
        let retired = p.take_retired();
        std::thread::spawn(move || drop(retired)).join().unwrap();
        drops.assert_worker_drop();
    }
}
