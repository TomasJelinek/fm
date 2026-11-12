//! Native tree actions retain exact occurrences; only backend workers resolve providers.
use super::*;

pub(super) enum Purpose {
    Clipboard,
    Properties,
    Menu(tree_shell::Menu),
    Open(TreeRowId),
    Paste,
}
impl Purpose {
    fn failure_title(&self) -> &'static str {
        match self {
            Self::Clipboard => "Clipboard unavailable",
            Self::Properties => "Properties unavailable",
            Self::Menu(_) => "Context menu unavailable",
            Self::Open(_) => "Cannot open item",
            Self::Paste => "Cannot paste files",
        }
    }
}

pub(super) struct Pending {
    pub capture: tree_capture::Capture,
    pub intent: FileClipboardIntent,
    pub scope: TreeClipboardScope,
    pub purpose: Purpose,
    pub source: Option<Arc<dyn CapturedClipboardFiles>>,
    pub desktop_request: Option<DesktopRequestId>,
}
impl Pending {
    fn command(
        &self,
        request: DesktopRequestId,
        source: Arc<dyn CapturedClipboardFiles>,
    ) -> Command {
        let cancel = self.capture.cancel.clone();
        match &self.purpose {
            Purpose::Clipboard => Command::CopyCapturedFiles {
                request,
                source,
                cancel,
            },
            Purpose::Properties => Command::ShowCapturedProperties {
                request,
                source,
                cancel,
            },
            Purpose::Menu(menu) => Command::ShowCapturedShellMenu {
                request,
                source,
                cancel,
                owner: menu.anchor.owner,
                position: menu.anchor.position,
                modifiers: menu.anchor.modifiers,
            },
            Purpose::Open(row) => Command::OpenCapturedEntry {
                request,
                pane: self.capture.view.stamp.pane,
                navigation: self.capture.view.stamp.navigation,
                row: *row,
                source,
                cancel,
            },
            Purpose::Paste => Command::ReadCapturedFileClipboard {
                request,
                source,
                cancel,
            },
        }
    }
}

impl Presenter {
    pub(super) fn request_tree_clipboard(
        &mut self,
        pane: PaneId,
        intent: FileClipboardIntent,
        send: &dyn Fn(Command) -> bool,
    ) {
        self.request_tree_action(
            pane,
            intent,
            TreeClipboardScope::SelectionOrFallback,
            Purpose::Clipboard,
            send,
        );
    }

    pub(super) fn request_tree_action(
        &mut self,
        pane: PaneId,
        intent: FileClipboardIntent,
        scope: TreeClipboardScope,
        purpose: Purpose,
        send: &dyn Fn(Command) -> bool,
    ) {
        if self.operations.closing || !self.native_action_allowed(pane) {
            return;
        }
        if self.file_actions.tree_clipboard.is_some() {
            self.desktop.report(
                "File action pending",
                "Wait for the current file action to finish.".into(),
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
        self.start_tree_action(capture, intent, scope, purpose, send);
    }

    pub(super) fn start_tree_action(
        &mut self,
        capture: tree_capture::Capture,
        intent: FileClipboardIntent,
        scope: TreeClipboardScope,
        purpose: Purpose,
        send: &dyn Fn(Command) -> bool,
    ) {
        if self.operations.closing || self.file_actions.tree_clipboard.is_some() {
            capture.cancel.cancel();
            self.retired.push(Box::new(capture));
            return;
        }
        self.file_actions.tree_clipboard = Some(Pending {
            capture,
            intent,
            scope,
            purpose,
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
                .expect("matched tree action");
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
                if let Purpose::Menu(menu) = &mut pending.purpose {
                    menu.single = publication
                        .view
                        .index
                        .row_positions
                        .get(&menu.row)
                        .is_some_and(|position| {
                            !publication.selection.contains_position(*position)
                                || publication.selection.len() == 1
                        });
                }
                if send(Command::Tree(Box::new(TreeCommand::Clipboard(
                    TreeClipboardRequest {
                        intent: pending.intent,
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
        if pending.desktop_request.is_none()
            && let Some(source) = &pending.source
        {
            let request = self.desktop.request();
            if send(pending.command(request, source.clone())) {
                pending.desktop_request = Some(request);
                if !matches!(pending.purpose, Purpose::Menu(_)) {
                    self.desktop.requests.insert(request);
                }
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
        let title = pending.purpose.failure_title();
        match result {
            Ok(source) if source.intent() == pending.intent => pending.source = Some(source),
            Ok(source) => {
                self.retired.push(Box::new(source));
                self.cancel_tree_clipboard(None);
                self.desktop
                    .report(title, "Captured file action changed.".into());
            }
            Err(error) => {
                self.cancel_tree_clipboard(None);
                self.desktop.report(title, error.as_str().into());
            }
        }
        self.flush_tree_clipboard(send);
    }

    pub(super) fn tree_clipboard_finished(&mut self, request: DesktopRequestId) {
        if self
            .file_actions
            .tree_clipboard
            .as_ref()
            .is_some_and(|pending| {
                pending.desktop_request == Some(request)
                    && !matches!(pending.purpose, Purpose::Menu(_))
            })
        {
            self.retired
                .push(Box::new(self.file_actions.tree_clipboard.take()));
        }
    }
}
