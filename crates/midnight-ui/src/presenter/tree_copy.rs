//! F5/F6 retain exact occurrences and intent while the backend captures provider ownership.
use super::*;

pub(super) struct Pending {
    intent: TransferIntent,
    capture: tree_capture::Capture,
    source: String,
    destination: PathBuf,
    summary: String,
}

impl Presenter {
    pub(super) fn request_tree_transfer(
        &mut self,
        pane: PaneId,
        intent: TransferIntent,
        send: &dyn Fn(Command) -> bool,
    ) {
        let p = &self.panes[pane.index()];
        let Some(capture) = tree_capture::Capture::new(p) else {
            return;
        };
        self.copies.tree_pending = Some(Pending {
            intent,
            capture,
            source: p.archive.as_ref().map_or_else(
                || p.display_path().to_string_lossy().into_owned(),
                |archive| archive.location.display_text(),
            ),
            destination: self.panes[pane.other().index()].display_path().to_owned(),
            summary: String::new(),
        });
        self.flush_tree_transfer(send);
    }

    pub(super) fn cancel_tree_transfer(&mut self, pane: Option<PaneId>) {
        if self
            .copies
            .tree_pending
            .as_ref()
            .is_some_and(|pending| pane.is_none_or(|pane| pending.capture.view.stamp.pane == pane))
        {
            let pending = self
                .copies
                .tree_pending
                .take()
                .expect("matched pending copy");
            pending.capture.cancel.cancel();
            self.retired.push(Box::new(pending));
        }
    }

    pub(super) fn flush_tree_transfer(&mut self, send: &dyn Fn(Command) -> bool) {
        let Some(mut pending) = self.copies.tree_pending.take() else {
            return;
        };
        let pane = pending.capture.view.stamp.pane;
        let p = &mut self.panes[pane.index()];
        let (request, publication) = match pending.capture.status(p, self.operations.closing) {
            tree_capture::Status::Ready(request, publication) => (request, publication),
            tree_capture::Status::Waiting => {
                self.copies.tree_pending = Some(pending);
                return;
            }
            status => {
                if matches!(status, tree_capture::Status::SelectionChanged) {
                    p.set_error(
                        "Selection changed before the transfer could capture it. Try again.",
                        &mut self.retired,
                    );
                }
                pending.capture.cancel.cancel();
                self.retired.push(Box::new(pending));
                return;
            }
        };
        let count = publication.selection.len();
        let named_row = if count == 0 {
            pending.capture.fallback
        } else if count == 1 {
            publication
                .selection
                .bits()
                .iter()
                .enumerate()
                .find(|(_, word)| **word != 0)
                .and_then(|(word, bits)| {
                    pending
                        .capture
                        .view
                        .rows
                        .get(word * 64 + bits.trailing_zeros() as usize)
                })
                .map(|row| row.id)
        } else {
            None
        };
        pending.summary = named_row
            .and_then(|id| pending.capture.view.index.row_positions.get(&id))
            .map_or_else(
                || {
                    format!(
                        "{} selected items",
                        self.number_locale.format_u64(count as u64)
                    )
                },
                |position| {
                    pending.capture.view.rows[*position]
                        .entry
                        .display_name
                        .clone()
                },
            );
        if send(Command::Tree(Box::new(TreeCommand::Transfer(
            TreeTransferRequest {
                intent: pending.intent,
                view: pending.capture.view.clone(),
                request,
                selection_revision: publication.selection_revision,
                selection: publication.selection.clone(),
                fallback: pending.capture.fallback,
                cancel: pending.capture.cancel.clone(),
            },
        )))) {
            p.tree.request = request;
            pending.capture.request = Some(request);
        }
        self.copies.tree_pending = Some(pending);
    }

    pub(super) fn tree_transfer_captured(
        &mut self,
        stamp: TreeViewStamp,
        request: TreeRequestId,
        result: Result<TreeTransferCapture, TreeDiagnostic>,
    ) {
        if !self.copies.tree_pending.as_ref().is_some_and(|pending| {
            pending.capture.view.stamp == stamp && pending.capture.request == Some(request)
        }) {
            self.retired.push(Box::new(result));
            return;
        }
        let pending = self
            .copies
            .tree_pending
            .take()
            .expect("matched pending copy");
        if self.operations.closing
            || self.copies.draft.is_some()
            || pending.capture.cancel.is_cancelled()
        {
            self.retired.push(Box::new(result));
        } else {
            match result {
                Ok(capture)
                    if capture.intent != pending.intent
                        || capture.sources.intent() != pending.intent =>
                {
                    self.retired.push(Box::new(capture));
                    self.desktop.report(
                        "Transfer unavailable",
                        "The captured operation does not match the requested Copy or Move.".into(),
                    );
                }
                Ok(capture) => {
                    let token = format!("copy-draft:{}", self.desktop.request().0);
                    self.copies.draft = Some(copy_dialog::CopyDraft {
                        token,
                        input: copy_dialog::CopyDraftInput::Tree(capture),
                        source: pending.source.clone(),
                        destination: pending.destination.clone(),
                        summary: pending.summary.clone(),
                        error: String::new(),
                        intent: pending.intent,
                    });
                }
                Err(error) => self.desktop.report(
                    if pending.intent == TransferIntent::Move {
                        "Move unavailable"
                    } else {
                        "Copy unavailable"
                    },
                    error.as_str().into(),
                ),
            }
        }
        self.retired.push(Box::new(pending));
    }
}
