//! Native clipboard capture; neither display paths nor UI code resolve providers.
use super::*;

pub(super) struct Pending {
    capture: tree_capture::Capture,
    intent: FileClipboardIntent,
    source: Option<Arc<dyn CapturedClipboardFiles>>,
    desktop_request: Option<DesktopRequestId>,
}

impl Presenter {
    pub(super) fn request_tree_clipboard(
        &mut self,
        pane: PaneId,
        intent: FileClipboardIntent,
        send: &dyn Fn(Command) -> bool,
    ) {
        if self.operations.closing || !self.native_action_allowed(pane) {
            return;
        }
        if self.file_actions.tree_clipboard.is_some() {
            self.desktop.report(
                "Clipboard busy",
                "A clipboard action is still pending.".into(),
            );
            return;
        }
        let p = &self.panes[pane.index()];
        if !p.search_allows_activation() {
            return;
        }
        let Some(capture) = tree_capture::Capture::new(p) else {
            return;
        };
        self.file_actions.tree_clipboard = Some(Pending {
            capture,
            intent,
            source: None,
            desktop_request: None,
        });
        self.flush_tree_clipboard(send);
    }

    pub(super) fn cancel_tree_clipboard(&mut self, pane: Option<PaneId>) {
        if self
            .file_actions
            .tree_clipboard
            .as_ref()
            .is_some_and(|pending| pane.is_none_or(|pane| pending.capture.view.stamp.pane == pane))
        {
            let pending = self
                .file_actions
                .tree_clipboard
                .take()
                .expect("matched clipboard");
            pending.capture.cancel.cancel();
            if let Some(request) = pending.desktop_request {
                self.desktop.requests.remove(&request);
            }
            self.retired.push(Box::new(pending));
        }
    }

    pub(super) fn flush_tree_clipboard(&mut self, send: &dyn Fn(Command) -> bool) {
        let Some(mut pending) = self.file_actions.tree_clipboard.take() else {
            return;
        };
        let p = &mut self.panes[pending.capture.view.stamp.pane.index()];
        match pending.capture.status(p, self.operations.closing) {
            tree_capture::Status::Gone | tree_capture::Status::SelectionChanged => {
                pending.capture.cancel.cancel();
                if let Some(request) = pending.desktop_request {
                    self.desktop.requests.remove(&request);
                }
                self.retired.push(Box::new(pending));
                return;
            }
            tree_capture::Status::Ready(request, publication) => {
                if send(Command::Tree(Box::new(TreeCommand::Clipboard(
                    TreeClipboardRequest {
                        intent: pending.intent,
                        scope: TreeClipboardScope::SelectionOrFallback,
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
        if pending.desktop_request.is_none()
            && let Some(source) = &pending.source
        {
            let request = self.desktop.request();
            if send(Command::CopyCapturedFiles {
                request,
                source: source.clone(),
                cancel: pending.capture.cancel.clone(),
            }) {
                pending.desktop_request = Some(request);
                self.desktop.requests.insert(request);
            }
        }
        self.file_actions.tree_clipboard = Some(pending);
    }

    pub(super) fn tree_clipboard_captured(
        &mut self,
        stamp: TreeViewStamp,
        request: TreeRequestId,
        result: Result<Arc<dyn CapturedClipboardFiles>, TreeDiagnostic>,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Some(pending) = self.file_actions.tree_clipboard.as_mut().filter(|pending| {
            pending.capture.view.stamp == stamp
                && pending.capture.request == Some(request)
                && pending.source.is_none()
        }) else {
            self.retired.push(Box::new(result));
            return;
        };
        match result {
            Ok(source) if source.intent() == pending.intent => pending.source = Some(source),
            Ok(source) => {
                self.retired.push(Box::new(source));
                self.cancel_tree_clipboard(None);
                self.desktop
                    .report("Clipboard unavailable", "Clipboard intent changed.".into());
            }
            Err(error) => {
                self.cancel_tree_clipboard(None);
                self.desktop
                    .report("Clipboard unavailable", error.as_str().into());
            }
        }
        self.flush_tree_clipboard(send);
    }

    pub(super) fn tree_clipboard_finished(&mut self, request: DesktopRequestId) {
        if self
            .file_actions
            .tree_clipboard
            .as_ref()
            .is_some_and(|pending| pending.desktop_request == Some(request))
        {
            self.retired
                .push(Box::new(self.file_actions.tree_clipboard.take()));
        }
    }
}
