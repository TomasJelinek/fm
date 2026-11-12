//! Native tree occurrences enter the ordinary deletion confirmation and progress flow.
use super::*;

pub(super) struct Pending {
    mode: DeleteMode,
    scope: TreeClipboardScope,
    capture: tree_capture::Capture,
}

impl Presenter {
    pub(super) fn request_tree_delete(
        &mut self,
        pane: PaneId,
        mode: DeleteMode,
        send: &dyn Fn(Command) -> bool,
    ) {
        if self.operations.closing || !self.native_action_allowed(pane) {
            return;
        }
        let p = &self.panes[pane.index()];
        if !p.search_allows_activation() {
            return;
        }
        let Some(capture) = tree_capture::Capture::new(p) else {
            return;
        };
        self.start_tree_delete_capture(
            mode,
            capture,
            TreeClipboardScope::SelectionOrFallback,
            send,
        );
    }

    pub(super) fn start_tree_delete_capture(
        &mut self,
        mode: DeleteMode,
        capture: tree_capture::Capture,
        scope: TreeClipboardScope,
        send: &dyn Fn(Command) -> bool,
    ) {
        let pane = capture.view.stamp.pane;
        let scoped_row_valid = match scope {
            TreeClipboardScope::SelectionOrFallback => true,
            TreeClipboardScope::Dragged(row) => capture
                .view
                .index
                .row_positions
                .get(&row)
                .and_then(|position| capture.view.rows.get(*position))
                .is_some_and(|row| row.selectable && !row.context_only),
        };
        let invalid = self.operations.closing
            || !self.native_action_allowed(pane)
            || capture.request.is_some()
            || !scoped_row_valid
            || !self.panes[pane.index()].search_allows_activation()
            || !self.panes[pane.index()]
                .tree_view()
                .is_some_and(|view| Arc::ptr_eq(view, &capture.view))
            || matches!(
                capture.status(&self.panes[pane.index()], false),
                tree_capture::Status::Gone | tree_capture::Status::SelectionChanged
            );
        if invalid {
            capture.cancel.cancel();
            self.retired.push(Box::new(capture));
            return;
        }
        self.cancel_tree_metadata(None);
        self.cancel_tree_rename(None);
        self.cancel_tree_delete(None);
        self.operations.tree_delete = Some(Pending {
            mode,
            scope,
            capture,
        });
        self.flush_tree_delete(send);
    }

    pub(super) fn cancel_tree_delete(&mut self, pane: Option<PaneId>) {
        if self
            .operations
            .tree_delete
            .as_ref()
            .is_some_and(|pending| pane.is_none_or(|pane| pending.capture.view.stamp.pane == pane))
        {
            let pending = self
                .operations
                .tree_delete
                .take()
                .expect("matched delete capture");
            pending.capture.cancel.cancel();
            self.retired.push(Box::new(pending));
        }
    }

    pub(super) fn flush_tree_delete(&mut self, send: &dyn Fn(Command) -> bool) {
        let Some(mut pending) = self.operations.tree_delete.take() else {
            return;
        };
        let p = &mut self.panes[pending.capture.view.stamp.pane.index()];
        if !p
            .tree_view()
            .is_some_and(|view| Arc::ptr_eq(view, &pending.capture.view))
        {
            pending.capture.cancel.cancel();
            self.retired.push(Box::new(pending));
            return;
        }
        match pending.capture.status(p, self.operations.closing) {
            tree_capture::Status::Gone | tree_capture::Status::SelectionChanged => {
                pending.capture.cancel.cancel();
                self.retired.push(Box::new(pending));
                return;
            }
            tree_capture::Status::Ready(request, publication) => {
                if send(Command::Tree(Box::new(TreeCommand::Delete(
                    TreeDeleteRequest {
                        mode: pending.mode,
                        scope: pending.scope,
                        view: pending.capture.view.clone(),
                        request,
                        selection_revision: publication.selection_revision,
                        selection: publication.selection,
                        fallback: pending.capture.fallback,
                        cancel: pending.capture.cancel.clone(),
                    },
                )))) {
                    p.tree.request = request;
                    pending.capture.request = Some(request);
                }
            }
            tree_capture::Status::Waiting => {}
        }
        self.operations.tree_delete = Some(pending);
    }

    pub(super) fn tree_delete_captured(
        &mut self,
        stamp: TreeViewStamp,
        request: TreeRequestId,
        result: Result<TreeDeleteCapture, TreeDiagnostic>,
        send: &dyn Fn(Command) -> bool,
    ) {
        if !self.operations.tree_delete.as_ref().is_some_and(|pending| {
            pending.capture.view.stamp == stamp && pending.capture.request == Some(request)
        }) {
            self.retired.push(Box::new(result));
            return;
        }
        self.flush_tree_delete(send);
        let Some(pending) = self.operations.tree_delete.take() else {
            self.retired.push(Box::new(result));
            return;
        };
        match result {
            Ok(captured)
                if captured.mode == pending.mode
                    && captured.root_count > 0
                    && captured.sources.stamp() == stamp
                    && captured.sources.request() == request
                    && captured.sources.mode() == pending.mode
                    && captured.sources.root_count() == captured.root_count =>
            {
                // Keep an owner for worker reclamation even if the sender drops a rejected command.
                self.retired.push(Box::new(captured.sources.clone()));
                if let Some(id) = self.next_operation_id(send) {
                    let source_directory = self.panes[stamp.pane.index()].path.clone();
                    if send(Command::PrepareCapturedDelete(PrepareCapturedDelete {
                        id,
                        sources: captured.sources,
                        source_directory: source_directory.clone(),
                    })) {
                        self.register_preparing_operation(
                            id,
                            MutationKind::Delete(pending.mode),
                            source_directory,
                        );
                    } else {
                        pending.capture.cancel.cancel();
                        self.desktop
                            .report("Operation queue busy", "Try again.".into());
                    }
                } else {
                    pending.capture.cancel.cancel();
                }
            }
            Ok(captured) => {
                pending.capture.cancel.cancel();
                self.retired.push(Box::new(captured));
                self.desktop
                    .report("Delete", "The captured deletion source changed.".into());
            }
            Err(error) => {
                pending.capture.cancel.cancel();
                self.desktop.report("Delete", error.as_str().into());
            }
        }
        self.retired.push(Box::new(pending));
    }
}
