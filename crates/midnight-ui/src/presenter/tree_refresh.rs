//! Coalesced read-only maintenance preserves an adopted native tree across refresh triggers.
use super::*;

#[derive(Clone, Copy)]
struct Sent {
    stamp: TreeViewStamp,
    request: TreeRequestId,
    epoch: u64,
}

#[derive(Default)]
pub(super) struct State {
    desired: u64,
    completed: u64,
    sent: Option<Sent>,
}

impl Pane {
    /// Manual refresh, native hints and transfers can affect any loaded native branch.
    /// The backend refreshes retained nodes, never the UI's current selection.
    pub(super) fn queue_native_tree_refresh(&mut self, retired: &mut Vec<DeferredDrop>) -> bool {
        if !self.native_capabilities() || self.archive.is_some() || self.tree_view().is_none() {
            return false;
        }
        if let Some(epoch) = self.tree.refresh.desired.checked_add(1) {
            self.tree.refresh.desired = epoch;
        } else {
            self.set_error(
                "Tree refresh sequence exhausted. Reopen this folder.",
                retired,
            );
        }
        true
    }
}

impl Presenter {
    pub(super) fn request_native_tree_refresh(
        &mut self,
        pane: PaneId,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        if !self.panes[pane.index()].queue_native_tree_refresh(&mut self.retired) {
            return false;
        }
        self.flush_selection(send);
        true
    }

    pub(super) fn flush_tree_refresh(&mut self, send: &dyn Fn(Command) -> bool) {
        if self.operations.closing {
            return;
        }
        for pane in [PaneId::Left, PaneId::Right] {
            let p = &mut self.panes[pane.index()];
            if p.loading
                || !p.tree_input_ready()
                || !p.tree.preview.is_empty()
                || self.copies.selection.pending_for(pane)
                || p.tree.refresh.sent.is_some()
                || p.tree.refresh.desired == p.tree.refresh.completed
            {
                continue;
            }
            let Some(view) = p.tree_view().cloned() else {
                continue;
            };
            let Some(request) = p.tree.request.0.checked_add(1).map(TreeRequestId) else {
                p.tree.refresh.completed = p.tree.refresh.desired;
                p.set_error(
                    "Tree request sequence exhausted. Reopen this folder.",
                    &mut self.retired,
                );
                continue;
            };
            let stamp = view.stamp;
            if send(Command::Tree(Box::new(TreeCommand::RefreshLoaded(
                TreeRefreshRequest { view, request },
            )))) {
                p.tree.request = request;
                p.tree.refresh.sent = Some(Sent {
                    stamp,
                    request,
                    epoch: p.tree.refresh.desired,
                });
            }
        }
    }

    pub(super) fn tree_refresh_finished(
        &mut self,
        stamp: TreeViewStamp,
        request: TreeRequestId,
        result: Result<(), TreeDiagnostic>,
        retryable: bool,
    ) {
        let p = &mut self.panes[stamp.pane.index()];
        if !p.tree_view().is_some_and(|view| {
            view.stamp.navigation == stamp.navigation && view.stamp.session == stamp.session
        }) {
            return;
        }
        let Some(sent) = p
            .tree
            .refresh
            .sent
            .filter(|sent| sent.stamp == stamp && sent.request == request)
        else {
            return;
        };
        p.tree.refresh.sent = None;
        if !retryable {
            // A newer refresh trigger retains its dirty epoch for another pass.
            p.tree.refresh.completed = sent.epoch;
            if let Err(error) = result {
                p.set_tree_error(&error, "Could not refresh the tree: ", &mut self.retired);
            }
        }
    }
}
