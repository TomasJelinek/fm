//! F4 captures one focused occurrence; display paths and selection are not authority.
use super::*;

pub(super) struct Pending {
    view: Arc<TreeProjection>,
    row: TreeRowId,
    request: Option<TreeRequestId>,
    cancel: Cancellation,
    editor: EditorPreferences,
    source: Option<Arc<dyn CapturedEditorFile>>,
    desktop_request: Option<DesktopRequestId>,
}

impl Presenter {
    pub(super) fn request_tree_editor(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        if self.operations.closing {
            return;
        }
        if self.editor.tree.is_some() {
            self.desktop.report(
                "Editor queue busy",
                "An editor request is still pending.".into(),
            );
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
            .filter(|row| row.selectable && !row.context_only && row.entry.kind == EntryKind::File)
        else {
            self.desktop.report(
                "Cannot open editor",
                "Focus a regular local file before pressing F4.".into(),
            );
            return;
        };
        self.editor.tree = Some(Pending {
            view: view.clone(),
            row: row.id,
            request: None,
            cancel: Cancellation::default(),
            editor: self.preferences.values.editor.clone(),
            source: None,
            desktop_request: None,
        });
        self.flush_tree_editor(send);
    }

    pub(super) fn cancel_tree_editor(&mut self, pane: Option<PaneId>) {
        if self
            .editor
            .tree
            .as_ref()
            .is_some_and(|pending| pane.is_none_or(|pane| pending.view.stamp.pane == pane))
        {
            let pending = self.editor.tree.take().expect("matched editor request");
            pending.cancel.cancel();
            if let Some(request) = pending.desktop_request {
                self.desktop.requests.remove(&request);
            }
            self.retired.push(Box::new(pending));
        }
    }

    pub(super) fn flush_tree_editor(&mut self, send: &dyn Fn(Command) -> bool) {
        let Some(mut pending) = self.editor.tree.take() else {
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
            if let Some(request) = pending.desktop_request {
                self.desktop.requests.remove(&request);
            }
            self.retired.push(Box::new(pending));
            return;
        }
        if pending.request.is_none() && p.tree_input_ready() {
            let Some(request) = p.tree.request.0.checked_add(1).map(TreeRequestId) else {
                pending.cancel.cancel();
                self.retired.push(Box::new(pending));
                return;
            };
            if send(Command::Tree(Box::new(TreeCommand::Edit(
                TreeEditorRequest {
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
        if pending.desktop_request.is_none()
            && let Some(source) = &pending.source
        {
            let request = self.desktop.request();
            if send(Command::EditCapturedFile {
                request,
                source: source.clone(),
                editor: pending.editor.clone(),
                cancel: pending.cancel.clone(),
            }) {
                pending.desktop_request = Some(request);
                self.desktop.requests.insert(request);
                self.desktop.status = "Opening external editor…".into();
            }
        }
        self.editor.tree = Some(pending);
    }

    pub(super) fn tree_editor_captured(
        &mut self,
        stamp: TreeViewStamp,
        request: TreeRequestId,
        result: Result<Arc<dyn CapturedEditorFile>, TreeDiagnostic>,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Some(pending) = self.editor.tree.as_mut().filter(|pending| {
            pending.view.stamp == stamp
                && pending.request == Some(request)
                && pending.source.is_none()
        }) else {
            self.retired.push(Box::new(result));
            return;
        };
        match result {
            Ok(source) if source.stamp() == stamp && source.row() == pending.row => {
                pending.source = Some(source)
            }
            Ok(source) => {
                self.retired.push(Box::new(source));
                self.cancel_tree_editor(None);
                self.desktop.report(
                    "Cannot open editor",
                    "The captured editor target changed.".into(),
                );
            }
            Err(error) => {
                self.cancel_tree_editor(None);
                self.desktop
                    .report("Cannot open editor", error.as_str().into());
            }
        }
        self.flush_tree_editor(send);
    }

    pub(super) fn tree_editor_finished(&mut self, request: DesktopRequestId) {
        if self
            .editor
            .tree
            .as_ref()
            .is_some_and(|pending| pending.desktop_request == Some(request))
        {
            self.retired.push(Box::new(self.editor.tree.take()));
        }
    }
}
