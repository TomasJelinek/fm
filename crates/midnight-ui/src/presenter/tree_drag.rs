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
}

pub(super) struct Destination {
    id: DragId,
    view: Arc<TreeProjection>,
    row: TreeRowId,
    request: TreeRequestId,
    target: Option<Arc<dyn CapturedDropDirectory>>,
}

impl Presenter {
    pub(super) fn begin_tree_drag(
        &mut self,
        pane: PaneId,
        row: usize,
        identity: &str,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let p = &self.panes[pane.index()];
        if !p.native_capabilities() || !p.tree_input_ready() || !p.search_allows_activation() {
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
        });
        self.flush_tree_drag(send);
        self.drag_drop.lifecycle.active().is_some()
    }

    pub(super) fn cancel_tree_drag(&mut self, pane: Option<PaneId>) {
        if self
            .drag_drop
            .tree
            .source
            .as_ref()
            .is_some_and(|source| pane.is_none_or(|pane| pane == source.capture.view.stamp.pane))
        {
            let source = self.drag_drop.tree.source.take().unwrap();
            source.capture.cancel.cancel();
            self.queue_cancelled_drag(source.id);
            self.retired.push(Box::new(source));
        }
        if self
            .drag_drop
            .tree
            .destination
            .as_ref()
            .is_some_and(|destination| pane.is_none_or(|pane| pane == destination.view.stamp.pane))
        {
            let destination = self.drag_drop.tree.destination.take().unwrap();
            self.queue_cancelled_drag(destination.id);
            self.retired.push(Box::new(destination));
        }
    }

    pub(super) fn finish_tree_drag(&mut self, id: DragId) {
        if self
            .drag_drop
            .tree
            .source
            .as_ref()
            .is_some_and(|source| source.id == id)
        {
            let source = self.drag_drop.tree.source.take().unwrap();
            source.capture.cancel.cancel();
            self.retired.push(Box::new(source));
        }
        if self
            .drag_drop
            .tree
            .destination
            .as_ref()
            .is_some_and(|target| target.id == id)
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
            source.capture.view.stamp == stamp && source.capture.request == Some(request)
        })
    }

    pub(super) fn tree_drag_captured(
        &mut self,
        result: Result<Arc<dyn CapturedClipboardFiles>, TreeDiagnostic>,
        send: &dyn Fn(Command) -> bool,
    ) {
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
        let mut result = Some(result);
        if self.tree_drag_preview_captured(stamp, request, id, &mut result) {
            return;
        }
        let result = result.expect("unhandled final drop receipt");
        let Some(pending) = self.drag_drop.tree.destination.as_mut().filter(|pending| {
            pending.id == id && pending.view.stamp == stamp && pending.request == request
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
