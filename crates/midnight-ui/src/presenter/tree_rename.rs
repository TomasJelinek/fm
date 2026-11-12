//! F2 keeps one exact focused native occurrence through backend preparation.
use super::*;

pub(super) struct Pending {
    view: Arc<TreeProjection>,
    row: TreeRowId,
    request: Option<TreeRequestId>,
    cancel: Cancellation,
    source: Option<Arc<dyn CapturedRenameTarget>>,
}
impl Presenter {
    pub(super) fn request_tree_rename(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        if self.operations.closing || !self.native_action_allowed(pane) {
            return;
        }
        let p = &self.panes[pane.index()];
        if p.loading || p.parent_cursor || !p.tree_input_ready() || !p.search_allows_activation() {
            return;
        }
        let Some(view) = p.tree_view() else {
            return;
        };
        let Some(row) = p
            .cursor
            .and_then(|id| view.index.directory.by_id.get(&id))
            .and_then(|position| view.rows.get(*position))
            .filter(|row| row.selectable && !row.context_only)
        else {
            return;
        };
        self.operations.tree_rename = Some(Pending {
            view: view.clone(),
            row: row.id,
            request: None,
            cancel: Cancellation::default(),
            source: None,
        });
        self.flush_tree_rename(send);
    }

    pub(super) fn cancel_tree_rename(&mut self, pane: Option<PaneId>) {
        if self
            .operations
            .tree_rename
            .as_ref()
            .is_some_and(|pending| pane.is_none_or(|pane| pending.view.stamp.pane == pane))
        {
            let pending = self
                .operations
                .tree_rename
                .take()
                .expect("matched Rename capture");
            pending.cancel.cancel();
            self.retired.push(Box::new(pending));
        }
    }

    pub(super) fn flush_tree_rename(&mut self, send: &dyn Fn(Command) -> bool) {
        let Some(mut pending) = self.operations.tree_rename.take() else {
            return;
        };
        let p = &mut self.panes[pending.view.stamp.pane.index()];
        if self.operations.closing
            || p.loading
            || pending.cancel.is_cancelled()
            || !p
                .tree_view()
                .is_some_and(|view| Arc::ptr_eq(view, &pending.view))
        {
            pending.cancel.cancel();
            self.retired.push(Box::new(pending));
            return;
        }
        if pending.request.is_none() && p.tree_input_ready() {
            let Some(request) = p.tree.request.0.checked_add(1).map(TreeRequestId) else {
                pending.cancel.cancel();
                self.retired.push(Box::new(pending));
                return;
            };
            if send(Command::Tree(Box::new(TreeCommand::Rename(
                TreeRenameRequest {
                    view: pending.view.clone(),
                    request,
                    row: pending.row,
                    cancel: pending.cancel.clone(),
                },
            )))) {
                p.tree.request = request;
                pending.request = Some(request);
            }
        }
        if let Some(source) = &pending.source {
            let source_directory = p.path.clone();
            if let Some(id) = self.next_operation_id(send) {
                // Pending retains the source throughout a rejected send; retry adds no owners.
                if send(Command::PrepareCapturedRename(PrepareCapturedRename {
                    id,
                    source: source.clone(),
                    source_directory: source_directory.clone(),
                })) {
                    self.register_preparing_operation(id, MutationKind::Rename, source_directory);
                    self.retired.push(Box::new(pending));
                    return;
                }
            }
        }
        self.operations.tree_rename = Some(pending);
    }

    pub(super) fn tree_rename_captured(
        &mut self,
        stamp: TreeViewStamp,
        request: TreeRequestId,
        result: Result<Arc<dyn CapturedRenameTarget>, TreeDiagnostic>,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Some(pending) = self.operations.tree_rename.as_mut().filter(|pending| {
            pending.view.stamp == stamp
                && pending.request == Some(request)
                && pending.source.is_none()
        }) else {
            self.retired.push(Box::new(result));
            return;
        };
        match result {
            Ok(source)
                if source.stamp() == stamp
                    && source.request() == request
                    && source.row() == pending.row =>
            {
                pending.source = Some(source)
            }
            Ok(source) => {
                self.retired.push(Box::new(source));
                self.cancel_tree_rename(None);
                self.desktop
                    .report("Rename", "The captured Rename target changed.".into());
            }
            Err(error) => {
                self.cancel_tree_rename(None);
                self.desktop.report("Rename", error.as_str().into());
            }
        }
        self.flush_tree_rename(send);
    }
}
