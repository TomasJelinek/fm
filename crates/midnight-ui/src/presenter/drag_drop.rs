//! Captured drag selections and admitted drops retain their identities under backpressure.
use super::*;
use crate::drag_lifecycle::{DragLifecycle, Phase, SourceStamp};
use std::collections::VecDeque;

#[derive(Default)]
pub(crate) struct DragDropState {
    pub(crate) lifecycle: DragLifecycle,
    pub(crate) targets: SharedDropTargets,
    pub(crate) host: Option<DragHost>,
    pub(crate) attached: bool,
    closed: bool,
    detached: bool,
    pub(crate) supported: Option<bool>,
    gesture_fallback: bool,
    pub(crate) pointer_down: bool,
    pub(crate) item_count: usize,
    pub(crate) hover: Option<PaneId>,
    cancelled: VecDeque<DragId>,
    accepted: VecDeque<(DropToken, OperationId)>,
    rejected: VecDeque<DropToken>,
    last_drop_token: u64,
}

pub(crate) fn invalidate(targets: &SharedDropTargets) {
    let mut snapshot = targets.lock().unwrap_or_else(|error| error.into_inner());
    if !snapshot.targets.is_empty() {
        snapshot.targets.clear();
        snapshot.revision = snapshot.revision.saturating_add(1);
    }
}

impl Presenter {
    fn drag_stamp(&self, pane: PaneId) -> SourceStamp {
        let p = &self.panes[pane.index()];
        SourceStamp {
            pane: pane.index(),
            navigation: p.generation,
            displayed: p.displayed_generation().0,
            view: p.search.view_revision,
        }
    }

    pub(crate) fn begin_file_drag(
        &mut self,
        pane: PaneId,
        row: usize,
        identity: &str,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        self.flush_drag_commands(send);
        if self.operations.closing
            || self.drag_drop.closed
            || !self.drag_drop.cancelled.is_empty()
            || self.drag_drop.lifecycle.active.is_some()
            || self.selection_actions[pane.index()].is_some()
        {
            return false;
        }
        let p = &self.panes[pane.index()];
        if !p.search_allows_activation()
            || p.selection_pending_bulk
            || p.selection_deferred
            || p.selection_rebase_pending
        {
            return false;
        }
        let Some(entry) = row
            .checked_sub(p.parent_offset())
            .and_then(|row| p.display_entry(row))
        else {
            return false;
        };
        let expected = format!(
            "{}:{}:{}:entry:{}",
            p.generation,
            p.displayed_generation().0,
            p.search.view_revision,
            entry.id.0
        );
        if identity != expected || !entry.path.is_absolute() {
            return false;
        }
        let selected = p.selection_visible(entry);
        let fallback = entry.path.clone();
        let view = p.display_index().clone();
        let Some(id) = self
            .drag_drop
            .lifecycle
            .begin(self.drag_stamp(pane))
            .map(DragId)
        else {
            return false;
        };
        self.drag_drop.pointer_down = true;
        self.drag_drop.gesture_fallback = false;
        self.drag_drop.item_count = 0;
        self.drag_drop.hover = None;
        self.active = pane;
        self.focus_display_row(pane, row);
        if selected {
            self.request_selection_action(pane, selection_actions::SelectionIntent::Drag(id), send);
        } else {
            let request = self.desktop.request();
            self.prepare_selected_drag(
                request,
                id,
                SelectedFiles {
                    selection: Arc::default(),
                    view,
                    fallback: Some(fallback),
                },
                send,
            );
        }
        self.drag_drop.lifecycle.active.is_some()
    }

    pub(super) fn prepare_selected_drag(
        &mut self,
        request: DesktopRequestId,
        id: DragId,
        files: SelectedFiles,
        send: &dyn Fn(Command) -> bool,
    ) {
        self.desktop.requests.remove(&request);
        let Some(gesture) = self.drag_drop.lifecycle.active.filter(|g| g.id == id.0) else {
            self.retired.push(Box::new(files));
            return;
        };
        let pane = if gesture.source.pane == 0 {
            PaneId::Left
        } else {
            PaneId::Right
        };
        if self.drag_stamp(pane) != gesture.source || !self.drag_drop.pointer_down {
            self.retired.push(Box::new(files));
            self.cancel_file_drag(send);
            return;
        }
        if !send(Command::PrepareDrag {
            id,
            pane,
            navigation: NavigationId(gesture.source.navigation),
            files,
        }) {
            self.cancel_file_drag(send);
            self.desktop.status = "Drag preparation queue busy. Try dragging again.".into();
        }
    }

    pub(crate) fn cancel_file_drag(&mut self, send: &dyn Fn(Command) -> bool) {
        if let Some(gesture) = self.drag_drop.lifecycle.cancel() {
            self.drag_drop.cancelled.push_back(DragId(gesture.id));
            let pane = if gesture.source.pane == 0 {
                PaneId::Left
            } else {
                PaneId::Right
            };
            if self.selection_actions[pane.index()]
                .as_ref()
                .is_some_and(|pending| {
                    matches!(pending.intent, selection_actions::SelectionIntent::Drag(id)
                    if id.0 == gesture.id)
                })
            {
                self.cancel_selection_actions(pane);
            }
        }
        self.drag_drop.pointer_down = false;
        self.drag_drop.hover = None;
        self.flush_drag_commands(send);
    }

    pub(crate) fn release_file_drag(
        &mut self,
        target: Option<(FileDropTarget, u64)>,
        send: &dyn Fn(Command) -> bool,
    ) {
        self.drag_drop.pointer_down = false;
        let Some(gesture) = self.drag_drop.lifecycle.release() else {
            return;
        };
        self.drag_drop.hover = None;
        let pane = if gesture.source.pane == 0 {
            PaneId::Left
        } else {
            PaneId::Right
        };
        let valid = gesture.phase == Phase::Ready && self.drag_stamp(pane) == gesture.source;
        if valid
            && let Some((target, targets_revision)) = target
            && send(Command::InternalDrop {
                id: DragId(gesture.id),
                target,
                targets_revision,
            })
        {
            return;
        }
        self.drag_drop.cancelled.push_back(DragId(gesture.id));
        if self.selection_actions[pane.index()]
            .as_ref()
            .is_some_and(|pending| {
                matches!(pending.intent, selection_actions::SelectionIntent::Drag(id)
                if id.0 == gesture.id)
            })
        {
            self.cancel_selection_actions(pane);
        }
        self.flush_drag_commands(send);
    }

    pub(crate) fn drag_native_ready(&self) -> Option<DragId> {
        self.drag_drop
            .lifecycle
            .active
            .filter(|gesture| {
                gesture.phase == Phase::Ready
                    && self.drag_drop.pointer_down
                    && self.drag_drop.supported == Some(true)
                    && !self.drag_drop.gesture_fallback
            })
            .map(|gesture| DragId(gesture.id))
    }

    pub(crate) fn arm_native_drag(&mut self, id: DragId) -> bool {
        if self.drag_native_ready() != Some(id) {
            return false;
        }
        self.drag_drop.lifecycle.native_started(id.0)
    }

    pub(crate) fn attach_drag_host(
        &mut self,
        native: NativeDragWindow,
        send: &dyn Fn(Command) -> bool,
    ) {
        if self.operations.closing || self.drag_drop.closed {
            return;
        }
        if self
            .drag_drop
            .host
            .as_ref()
            .is_some_and(|host| host.native == native && !host.lifetime.is_cancelled())
        {
            return;
        }
        self.cancel_file_drag(send);
        let generation = self.drag_drop.host.as_ref().map_or(1, |host| {
            host.lifetime.cancel();
            host.generation.saturating_add(1)
        });
        invalidate(&self.drag_drop.targets);
        self.drag_drop
            .targets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .host_generation = generation;
        self.drag_drop.host = Some(DragHost {
            generation,
            native,
            lifetime: Cancellation::default(),
        });
        self.drag_drop.attached = false;
        self.drag_drop.supported = (native == NativeDragWindow::Unsupported).then_some(false);
        self.flush_drag_commands(send);
    }

    pub(crate) fn publish_drag_targets(&mut self, rectangles: [Option<DropRect>; 2]) {
        let Some(host) = self
            .drag_drop
            .host
            .as_ref()
            .filter(|host| !host.lifetime.is_cancelled())
        else {
            return;
        };
        let wanted: [_; 2] = std::array::from_fn(|index| {
            let pane = if index == 0 {
                PaneId::Left
            } else {
                PaneId::Right
            };
            let p = &self.panes[index];
            let rect = rectangles[index]?;
            let navigation = p.successful_generation?;
            (!self.operations.closing
                && !self.drag_drop.closed
                && !p.loading
                && p.native_capabilities()
                && p.path.is_absolute())
            .then_some((pane, navigation, p.path.as_path(), rect))
        });
        let mut snapshot = self
            .drag_drop
            .targets
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if snapshot.host_generation == host.generation
            && snapshot.targets.len() == wanted.iter().flatten().count()
            && snapshot
                .targets
                .iter()
                .zip(wanted.iter().flatten())
                .all(|(old, current)| {
                    old.pane == current.0
                        && old.navigation == current.1
                        && old.directory.as_path() == current.2
                        && old.rect == current.3
                })
        {
            return;
        }
        let targets = wanted
            .into_iter()
            .flatten()
            .map(|(pane, navigation, path, rect)| {
                let directory = snapshot
                    .targets
                    .iter()
                    .find(|old| old.pane == pane && old.directory.as_path() == path)
                    .map_or_else(|| Arc::new(path.to_owned()), |old| old.directory.clone());
                FileDropTarget {
                    pane,
                    navigation,
                    directory,
                    rect,
                }
            })
            .collect();
        snapshot.host_generation = host.generation;
        if let Some(revision) = snapshot.revision.checked_add(1) {
            snapshot.revision = revision;
            snapshot.targets = targets;
            if snapshot.validate().is_err() {
                snapshot.targets.clear();
            }
        } else {
            snapshot.targets.clear();
        }
    }

    pub(crate) fn flush_drag_commands(&mut self, send: &dyn Fn(Command) -> bool) {
        if let Some(gesture) = self.drag_drop.lifecycle.active {
            let pane = if gesture.source.pane == 0 {
                PaneId::Left
            } else {
                PaneId::Right
            };
            if let Some(stale) = self.drag_drop.lifecycle.validate(self.drag_stamp(pane)) {
                self.drag_drop.cancelled.push_back(DragId(stale.id));
                self.drag_drop.pointer_down = false;
                self.drag_drop.hover = None;
            }
        }
        if !self.drag_drop.attached
            && !self.drag_drop.closed
            && !self.operations.closing
            && let Some(host) = &self.drag_drop.host
            && !host.lifetime.is_cancelled()
            && send(Command::AttachDragDrop {
                host: host.clone(),
                targets: self.drag_drop.targets.clone(),
            })
        {
            self.drag_drop.attached = true;
        }
        while let Some(id) = self.drag_drop.cancelled.front().copied() {
            if !send(Command::CancelDrag(id)) {
                break;
            }
            self.drag_drop.cancelled.pop_front();
        }
        while let Some(token) = self.drag_drop.rejected.front().copied() {
            if !send(Command::RejectDrop(token)) {
                break;
            }
            self.drag_drop.rejected.pop_front();
        }
        while let Some((token, id)) = self.drag_drop.accepted.front().copied() {
            if !send(Command::AcceptDrop { token, id }) {
                break;
            }
            self.drag_drop.accepted.pop_front();
        }
    }

    pub(super) fn flush_admitted_drops_before_copy(
        &mut self,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        // The copy worker accepts monotonically increasing operation IDs. A later paste
        // cannot pass an earlier admitted drop whose handoff is still backpressured.
        self.flush_drag_commands(send);
        self.drag_drop.accepted.is_empty()
    }

    pub(super) fn cancel_pending_drop(
        &mut self,
        id: OperationId,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let Some(index) = self
            .drag_drop
            .accepted
            .iter()
            .position(|(_, job)| *job == id)
        else {
            return false;
        };
        let (token, _) = self.drag_drop.accepted.remove(index).unwrap();
        self.drag_drop.rejected.push_back(token);
        if let Some(progress) = self.copies.progress.iter_mut().find(|p| p.id == id) {
            Arc::make_mut(progress).state = CopyOperationState::Cancelled;
        }
        self.copies.conflicts.retain(|conflict| conflict.id != id);
        self.trim_copy_history();
        self.flush_drag_commands(send);
        true
    }

    pub fn shutdown_drag_drop(&mut self, send: &dyn Fn(Command) -> bool) {
        self.retire_drag_drop();
        for (token, id) in self.drag_drop.accepted.drain(..) {
            self.drag_drop.rejected.push_back(token);
            if let Some(progress) = self.copies.progress.iter_mut().find(|p| p.id == id) {
                Arc::make_mut(progress).state = CopyOperationState::Cancelled;
            }
        }
        self.trim_copy_history();
        self.cancel_file_drag(send);
        if !self.drag_drop.detached
            && let Some(host) = &self.drag_drop.host
        {
            self.drag_drop.detached = send(Command::DetachDragDrop {
                host_generation: host.generation,
            });
        }
    }

    pub(super) fn retire_drag_drop(&mut self) {
        self.drag_drop.closed = true;
        if let Some(host) = &self.drag_drop.host {
            host.lifetime.cancel();
        }
        invalidate(&self.drag_drop.targets);
    }

    pub(super) fn dispatch_drag_event(
        &mut self,
        event: Event,
        send: &dyn Fn(Command) -> bool,
    ) -> Option<Event> {
        let Event::DragDrop(event) = event else {
            return Some(event);
        };
        match event {
            DragDropEvent::Ready {
                host_generation,
                supported,
            } => {
                if self.drag_drop.host.as_ref().is_some_and(|host| {
                    host.generation == host_generation && !host.lifetime.is_cancelled()
                }) {
                    self.drag_drop.supported = Some(supported);
                }
            }
            DragDropEvent::Hover {
                host_generation,
                pane,
            } => {
                let targets = self
                    .drag_drop
                    .targets
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                if targets.host_generation == host_generation {
                    self.drag_drop.hover =
                        pane.filter(|pane| targets.targets.iter().any(|t| t.pane == *pane));
                }
            }
            DragDropEvent::Prepared { id, item_count } => {
                self.flush_drag_commands(send);
                if self.drag_drop.pointer_down && self.drag_drop.lifecycle.prepared(id.0) {
                    self.drag_drop.item_count = item_count;
                } else if !self
                    .drag_drop
                    .lifecycle
                    .active
                    .is_some_and(|g| g.id == id.0)
                {
                    self.drag_drop.cancelled.push_back(id);
                }
            }
            DragDropEvent::Finished { id, outcome } => {
                if self
                    .drag_drop
                    .lifecycle
                    .active
                    .is_some_and(|g| g.id == id.0)
                {
                    if outcome == DragFinish::Unavailable && self.drag_drop.pointer_down {
                        self.drag_drop.gesture_fallback = true;
                        self.drag_drop.lifecycle.active.as_mut().unwrap().phase = Phase::Ready;
                        self.desktop.status =
                            "External dragging is unavailable here; copy between panels.".into();
                    } else {
                        self.drag_drop.lifecycle.cancel();
                        self.drag_drop.pointer_down = false;
                        self.drag_drop.hover = None;
                        if outcome == DragFinish::Unavailable {
                            self.drag_drop.cancelled.push_back(id);
                        }
                    }
                }
            }
            DragDropEvent::DropAccepted {
                token,
                pane: _,
                item_count,
                destination,
            } => {
                if token.0 <= self.drag_drop.last_drop_token {
                    return None;
                }
                self.drag_drop.last_drop_token = token.0;
                let next = self.operations.next_id.checked_add(1);
                if self.operations.closing
                    || self.drag_drop.closed
                    || item_count == 0
                    || !destination.is_absolute()
                    || self.drag_drop.accepted.len() >= 32
                {
                    self.drag_drop.rejected.push_back(token);
                } else if let Some(next) = next {
                    let id = OperationId(next);
                    self.operations.next_id = id.0;
                    self.copies.progress.push(Arc::new(CopyProgress::queued(
                        id,
                        (*destination).clone(),
                        item_count as u64,
                        TransferIntent::Copy,
                    )));
                    self.drag_drop.accepted.push_back((token, id));
                } else {
                    self.drag_drop.rejected.push_back(token);
                }
            }
            DragDropEvent::Failed { id, error } => {
                if id.is_some_and(|id| {
                    self.drag_drop
                        .lifecycle
                        .active
                        .is_some_and(|g| g.id == id.0)
                }) {
                    self.cancel_file_drag(send);
                }
                if id.is_none() {
                    self.drag_drop.supported = Some(false);
                }
                self.desktop.status = format!("Drag/drop: {error}");
            }
        }
        self.flush_drag_commands(send);
        None
    }
}
