//! Speculative drag feedback owns cancellation only; it never authorizes a drop.
use super::*;

#[derive(Default)]
pub(crate) struct State {
    revision: u64,
    pending: Option<Pending>,
}
struct Pending {
    request: InternalDropPreviewRequest,
    sent: bool,
    target: Option<Arc<dyn CapturedDropDirectory>>,
    completed: bool,
    captured: bool,
    intent: Option<TransferIntent>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    Pending,
    Allowed(TransferIntent),
    Rejected,
}

fn same_destination(left: &InternalDropDestination, right: &InternalDropDestination) -> bool {
    match (left, right) {
        (
            InternalDropDestination::PaneRoot {
                pane: a,
                navigation: an,
            },
            InternalDropDestination::PaneRoot {
                pane: b,
                navigation: bn,
            },
        ) => a == b && an == bn,
        (
            InternalDropDestination::PanelEntry {
                pane: a,
                navigation: an,
                entry: ae,
            },
            InternalDropDestination::PanelEntry {
                pane: b,
                navigation: bn,
                entry: be,
            },
        ) => a == b && an == bn && ae == be,
        (
            InternalDropDestination::TreeOccurrence {
                view: a, row: ar, ..
            },
            InternalDropDestination::TreeOccurrence {
                view: b, row: br, ..
            },
        ) => Arc::ptr_eq(a, b) && ar == br,
        _ => false,
    }
}

impl Presenter {
    pub(crate) fn cancel_drag_preview(&mut self) {
        if let Some(pending) = self.drag_drop.preview.pending.take() {
            pending.request.cancel.cancel();
            self.retired.push(Box::new(pending));
        }
    }

    pub(crate) fn update_drag_preview(
        &mut self,
        destination: Option<InternalDropDestination>,
        intent: InternalDropIntent,
        send: &dyn Fn(Command) -> bool,
    ) {
        let gesture = self.drag_drop.lifecycle.active().filter(|gesture| {
            gesture.phase == Phase::Ready
                && self.drag_drop.pointer_down
                && self.drag_stamp(if gesture.source.pane == 0 {
                    PaneId::Left
                } else {
                    PaneId::Right
                }) == gesture.source
        });
        let (Some(gesture), Some(mut destination)) = (gesture, destination) else {
            self.cancel_drag_preview();
            return;
        };
        let same = self
            .drag_drop
            .preview
            .pending
            .as_ref()
            .is_some_and(|pending| {
                pending.request.id == DragId(gesture.id)
                    && pending.request.intent == intent
                    && !pending.request.cancel.is_cancelled()
                    && same_destination(&pending.request.destination, &destination)
            });
        if !same {
            self.cancel_drag_preview();
            let Some(revision) = self.drag_drop.preview.revision.checked_add(1) else {
                return;
            };
            self.drag_drop.preview.revision = revision;
            if let InternalDropDestination::TreeOccurrence { view, request, .. } = &mut destination
            {
                let pane = &mut self.panes[view.stamp.pane.index()];
                let Some(next) = pane.tree.request.0.checked_add(1) else {
                    return;
                };
                pane.tree.request = TreeRequestId(next);
                *request = pane.tree.request;
            }
            self.drag_drop.preview.pending = Some(Pending {
                request: InternalDropPreviewRequest {
                    id: DragId(gesture.id),
                    revision: DragPreviewRevision(revision),
                    destination,
                    intent,
                    cancel: Cancellation::default(),
                },
                sent: false,
                target: None,
                completed: false,
                captured: false,
                intent: None,
            });
        }
        let pending = self
            .drag_drop
            .preview
            .pending
            .as_mut()
            .expect("current hover request");
        if !pending.sent {
            pending.sent = send(Command::PreviewInternalDrop(Box::new(
                pending.request.clone(),
            )));
        }
        if pending.sent
            && let Some(target) = &pending.target
            && send(Command::CompleteInternalDropPreview {
                id: pending.request.id,
                revision: pending.request.revision,
                target: target.clone(),
            })
        {
            self.retired.push(Box::new(pending.target.take()));
        }
    }

    pub(crate) fn drag_preview_status(&self) -> Status {
        match self.drag_drop.preview.pending.as_ref() {
            Some(pending) if !pending.completed => Status::Pending,
            Some(pending) if pending.intent.is_some() => Status::Allowed(pending.intent.unwrap()),
            _ => Status::Rejected,
        }
    }

    pub(super) fn apply_drag_preview(
        &mut self,
        id: DragId,
        revision: DragPreviewRevision,
        intent: Option<TransferIntent>,
    ) {
        if let Some(pending) = self.drag_drop.preview.pending.as_mut()
            && pending.request.id == id
            && pending.request.revision == revision
            && !pending.completed
            && !pending.request.cancel.is_cancelled()
            && self
                .drag_drop
                .lifecycle
                .active()
                .is_some_and(|gesture| gesture.id == id.0 && gesture.phase == Phase::Ready)
        {
            pending.completed = true;
            pending.intent = intent;
        }
    }

    pub(crate) fn tree_drag_preview_captured(
        &mut self,
        stamp: TreeViewStamp,
        request: TreeRequestId,
        id: DragId,
        result: &mut Option<Result<Arc<dyn CapturedDropDirectory>, TreeDiagnostic>>,
    ) -> bool {
        let Some(pending) = self.drag_drop.preview.pending.as_mut().filter(|pending| {
            pending.request.id == id && !pending.request.cancel.is_cancelled()
                && matches!(&pending.request.destination, InternalDropDestination::TreeOccurrence { view, request: expected, .. } if view.stamp == stamp && *expected == request)
        }) else { return false; };
        let result = result.take().expect("unhandled tree receipt");
        if pending.captured || pending.completed {
            self.retired.push(Box::new(result));
            return true;
        }
        pending.captured = true;
        match result {
            Ok(target)
                if !pending.completed
                    && pending.target.is_none()
                    && target.stamp() == stamp
                    && matches!(&pending.request.destination, InternalDropDestination::TreeOccurrence { row, .. } if *row == target.row()) =>
            {
                pending.target = Some(target)
            }
            result => {
                if !pending.completed {
                    pending.completed = true;
                    pending.intent = None;
                }
                self.retired.push(Box::new(result));
            }
        }
        true
    }
}
