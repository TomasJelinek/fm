//! Ctrl+F8 captures acknowledged occurrences before opening the existing checksum window.
use super::*;

pub(super) struct Pending {
    capture: tree_capture::Capture,
}

impl Presenter {
    pub(super) fn request_tree_checksums(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        if self.operations.closing || !self.native_action_allowed(pane) {
            return;
        }
        if self.checksums.tree.is_some() {
            self.desktop
                .report("Checksums", "A checksum capture is still pending.".into());
            return;
        }
        if self.checksums.sessions.len() >= checksums::MAX_CHECKSUM_WINDOWS {
            self.desktop.report(
                "Checksums",
                "Close one of the four checksum windows first.".into(),
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
        self.checksums.tree = Some(Pending { capture });
        self.flush_tree_checksums(send);
    }

    pub(super) fn cancel_tree_checksums(&mut self, pane: Option<PaneId>) {
        if self
            .checksums
            .tree
            .as_ref()
            .is_some_and(|pending| pane.is_none_or(|pane| pending.capture.view.stamp.pane == pane))
        {
            let pending = self
                .checksums
                .tree
                .take()
                .expect("matched checksum capture");
            pending.capture.cancel.cancel();
            self.retired.push(Box::new(pending));
        }
    }

    pub(super) fn flush_tree_checksums(&mut self, send: &dyn Fn(Command) -> bool) {
        let Some(mut pending) = self.checksums.tree.take() else {
            return;
        };
        let p = &mut self.panes[pending.capture.view.stamp.pane.index()];
        // A later projection cannot approve an earlier selection, even after enqueue.
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
                if send(Command::Tree(Box::new(TreeCommand::Checksums(
                    TreeChecksumRequest {
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
        self.checksums.tree = Some(pending);
    }

    pub(super) fn tree_checksums_captured(
        &mut self,
        stamp: TreeViewStamp,
        request: TreeRequestId,
        result: Result<TreeChecksumCapture, TreeDiagnostic>,
        send: &dyn Fn(Command) -> bool,
    ) {
        if !self.checksums.tree.as_ref().is_some_and(|pending| {
            pending.capture.view.stamp == stamp && pending.capture.request == Some(request)
        }) {
            self.retired.push(Box::new(result));
            return;
        }
        self.flush_tree_checksums(send);
        let Some(pending) = self.checksums.tree.take() else {
            self.retired.push(Box::new(result));
            return;
        };
        match result {
            Ok(captured)
                if captured.sources.stamp() == stamp && captured.sources.request() == request =>
            {
                let files = SelectedFiles {
                    selection: Arc::new(SelectionSnapshot::default()),
                    view: Arc::new(DirectoryIndex::default()),
                    fallback: captured.fallback,
                };
                self.open_checksum_sources(captured.directory, files, Some(captured.sources), send);
            }
            Ok(captured) => {
                pending.capture.cancel.cancel();
                self.retired.push(Box::new(captured));
                self.desktop
                    .report("Checksums", "The captured checksum source changed.".into());
            }
            Err(error) => {
                pending.capture.cancel.cancel();
                self.desktop.report("Checksums", error.as_str().into());
            }
        }
        self.retired.push(Box::new(pending));
    }
}
