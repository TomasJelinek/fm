//! Exact tree argument capture before common Custom Commands preview or launch.
use super::*;

pub(super) struct Pending {
    capture: tree_capture::Capture,
    command: user_commands::CommandCapture,
    defer_retry: bool,
}

impl Presenter {
    pub(super) fn request_tree_user_command(
        &mut self,
        pane: PaneId,
        command: user_commands::CommandCapture,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        if !self.native_action_allowed(pane) || self.operations.closing {
            self.retired.push(Box::new(command));
            return false;
        }
        if self.command_launches.tree[pane.index()].is_some() {
            self.report_user_command_error(
                command.settings_preview.as_ref(),
                "Custom Commands",
                "A command is already waiting for this panel's captured selection.".into(),
            );
            self.retired.push(Box::new(command));
            return false;
        }
        let Some(capture) = tree_capture::Capture::optional_targets(&self.panes[pane.index()])
        else {
            self.retired.push(Box::new(command));
            return false;
        };
        self.command_launches.tree[pane.index()] = Some(Pending {
            capture,
            command,
            defer_retry: false,
        });
        self.flush_tree_user_commands(send);
        true
    }

    pub(super) fn cancel_tree_user_commands(&mut self, pane: Option<PaneId>) {
        for id in [PaneId::Left, PaneId::Right] {
            if pane.is_none_or(|pane| pane == id)
                && let Some(pending) = self.command_launches.tree[id.index()].take()
            {
                pending.capture.cancel.cancel();
                self.retired.push(Box::new(pending));
            }
        }
    }

    pub(super) fn flush_tree_user_commands(&mut self, send: &dyn Fn(Command) -> bool) {
        for pane in [PaneId::Left, PaneId::Right] {
            let Some(mut pending) = self.command_launches.tree[pane.index()].take() else {
                continue;
            };
            let p = &mut self.panes[pane.index()];
            let current = p
                .tree_view()
                .is_some_and(|view| Arc::ptr_eq(view, &pending.capture.view))
                && pending
                    .command
                    .settings_preview
                    .as_ref()
                    .is_none_or(|origin| origin.matches(&self.user_command_settings));
            if !current
                || matches!(
                    pending.capture.status(p, self.operations.closing),
                    tree_capture::Status::Gone | tree_capture::Status::SelectionChanged
                )
            {
                pending.capture.cancel.cancel();
                self.retired.push(Box::new(pending));
                continue;
            }
            if pending.defer_retry {
                pending.defer_retry = false;
                self.command_launches.tree[pane.index()] = Some(pending);
                continue;
            }
            if let tree_capture::Status::Ready(request, publication) =
                pending.capture.status(p, false)
                && send(Command::Tree(Box::new(TreeCommand::UserCommand(
                    TreeUserCommandRequest {
                        view: pending.capture.view.clone(),
                        request,
                        selection_revision: publication.selection_revision,
                        selection: publication.selection,
                        focus: pending.capture.fallback,
                        cancel: pending.capture.cancel.clone(),
                    },
                ))))
            {
                p.tree.request = request;
                pending.capture.request = Some(request);
            }
            if pending.command.request.captured.is_some() {
                if self.command_launches.next_submission == u64::MAX
                    || self
                        .command_launches
                        .jobs
                        .iter()
                        .filter(|job| !job.finished)
                        .count()
                        >= 32
                {
                    self.report_user_command_error(
                        pending.command.settings_preview.as_ref(),
                        "Custom Commands",
                        "The command launch queue is full or unavailable. Retry explicitly.".into(),
                    );
                    pending.capture.cancel.cancel();
                    self.retired.push(Box::new(pending));
                    continue;
                }
                if self.submit_user_command_captured(pending.command.clone(), None, true, send) {
                    // Accepted work retains this cancellation independently of later panel navigation.
                    self.retired.push(Box::new(pending));
                    continue;
                }
                pending.defer_retry = true;
            }
            self.command_launches.tree[pane.index()] = Some(pending);
        }
    }

    pub(super) fn tree_user_command_captured(
        &mut self,
        stamp: TreeViewStamp,
        request: TreeRequestId,
        retryable: bool,
        result: Result<Arc<dyn CapturedUserCommandSources>, TreeDiagnostic>,
        send: &dyn Fn(Command) -> bool,
    ) {
        self.flush_tree_user_commands(send);
        let Some(pending) = &mut self.command_launches.tree[stamp.pane.index()] else {
            self.retired.push(Box::new(result));
            return;
        };
        if pending.capture.view.stamp != stamp
            || pending.capture.request != Some(request)
            || pending.command.request.captured.is_some()
        {
            self.retired.push(Box::new(result));
            return;
        }
        match result {
            Ok(source) if source.stamp() == stamp && source.request() == request => {
                pending.command.request.captured = Some(source);
            }
            Ok(source) => {
                self.retired.push(Box::new(source));
                self.cancel_tree_user_commands(Some(stamp.pane));
                self.desktop.report(
                    "Custom Commands",
                    "The captured command source changed.".into(),
                );
                return;
            }
            Err(_) if retryable => {
                pending.capture.request = None;
                pending.defer_retry = true;
                return;
            }
            Err(error) => {
                let origin = pending.command.settings_preview.clone();
                self.cancel_tree_user_commands(Some(stamp.pane));
                self.report_user_command_error(
                    origin.as_ref(),
                    "Custom Commands",
                    error.as_str().into(),
                );
                return;
            }
        }
        self.flush_tree_user_commands(send);
    }
}
