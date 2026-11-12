use super::*;
use crate::tree_actor::{BranchCompletion, BranchJob};
use std::{collections::VecDeque, sync::Mutex};
use tokio::sync::Notify;

#[derive(Clone)]
pub(super) struct Generation {
    pub navigation: NavigationId,
    pub sort: SortSpec,
    pub cancel: Cancellation,
}
#[derive(Default)]
struct Inbox {
    commands: VecDeque<TreeCommand>,
    size_ack: Option<(midnight_core::TreeViewStamp, u64)>,
    generation: Option<Generation>,
    reset: bool,
    ready: Option<NavigationId>,
    completion: Option<BranchCompletion>,
    published: Weak<TreeProjection>,
    acknowledged: Weak<TreeProjection>,
    published_stamp: Option<midnight_core::TreeViewStamp>,
    closed: bool,
}
#[derive(Default)]
pub(super) struct Mailbox {
    inner: Mutex<Inbox>,
    pub wake: Arc<Notify>,
}
pub(super) enum Input {
    Reset(Generation),
    Ready(NavigationId),
    Command(TreeCommand),
    Completion(BranchCompletion),
    Closed,
}
#[derive(Debug)]
pub(super) enum PushError {
    Full(TreeCommand),
    Rejected(TreeCommand),
    Superseded(TreeCommand),
}
impl Mailbox {
    /// Submit from the model worker; queued work retains its existing branch permit.
    pub fn start_branch(self: &Arc<Self>, job: BranchJob) -> tokio::task::JoinHandle<()> {
        let inbox = self.clone();
        tokio::task::spawn_blocking(move || inbox.complete(job.run()))
    }
    /// Returns at most one replaced observation for awaited worker-side retirement.
    pub fn push(&self, command: TreeCommand) -> Result<Option<TreeCommand>, PushError> {
        let mut inbox = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let TreeCommand::Sizes(midnight_core::TreeSizeCommand::Acknowledge { stamp, version }) =
            &command
        {
            // One allocation-free ACK slot cannot be saturated by ordinary tree
            // commands. Old-generation ACKs have no state left to release.
            if !inbox.closed
                && inbox
                    .generation
                    .as_ref()
                    .is_some_and(|generation| generation.navigation == stamp.navigation)
            {
                if inbox.size_ack.is_none_or(|(old, previous)| {
                    old.session != stamp.session || previous <= *version
                }) {
                    inbox.size_ack = Some((*stamp, *version));
                }
                self.wake.notify_one();
            }
            return Ok(None);
        }
        if inbox.closed
            || inbox
                .generation
                .as_ref()
                .is_none_or(|generation| generation.navigation != command_navigation(&command))
        {
            return Err(PushError::Rejected(command));
        }
        if inbox
            .published_stamp
            .is_some_and(|stamp| superseded_observation(&command, stamp))
        {
            // No command is admitted. The controller retires this old view on a
            // worker; the latest publication will trigger fresh UI observations.
            return Err(PushError::Superseded(command));
        }
        if !known_view(&command, &inbox.published, &inbox.acknowledged) {
            return Err(PushError::Rejected(command));
        }
        let viewport = matches!(command, TreeCommand::Viewport { .. });
        let scope = match &command {
            TreeCommand::Enable { .. } => None,
            TreeCommand::Sizes(command) => Some(size_command_stamp(command)),
            TreeCommand::Commit(view)
            | TreeCommand::Viewport { view, .. }
            | TreeCommand::Branch { view, .. }
            | TreeCommand::Navigate { view, .. } => Some(view.stamp),
            // Every other view-bearing command passed the exact latest Arc check above.
            _ => inbox.published_stamp,
        };
        let replaced = if viewport || inbox.commands.len() >= 8 {
            scope.and_then(|scope| {
                inbox.commands.iter().position(|pending| {
                    matches!(pending, TreeCommand::Viewport { view, .. }
                    if view.stamp.pane == scope.pane
                        && view.stamp.navigation == scope.navigation
                        && view.stamp.session == scope.session)
                })
            })
        } else {
            None
        };
        let retired = replaced.and_then(|position| inbox.commands.remove(position));
        if inbox.commands.len() >= 8 {
            // Optional read-ahead must neither displace an explicit intent nor
            // turn its own backpressure into a visible action failure.
            return Err(if viewport {
                PushError::Superseded(command)
            } else {
                PushError::Full(command)
            });
        }
        if let TreeCommand::Commit(view) = &command {
            // Admission proved this is the exact emitted Arc. Keep a weak receipt
            // for row gestures queued while the next publication travels to the UI.
            inbox.acknowledged = Arc::downgrade(view);
        }
        inbox.commands.push_back(command);
        self.wake.notify_one();
        Ok(retired)
    }
    pub fn publish(&self, view: &Arc<TreeProjection>) -> bool {
        let mut inbox = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inbox.closed
            || inbox.generation.as_ref().is_none_or(|generation| {
                generation.navigation != view.stamp.navigation || generation.cancel.is_cancelled()
            })
        {
            return false;
        }
        inbox.published = Arc::downgrade(view);
        inbox.published_stamp = Some(view.stamp);
        true
    }
    pub fn reset(&self, generation: Generation) {
        let mut inbox = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(old) = &inbox.generation {
            if generation.navigation <= old.navigation {
                return;
            }
            old.cancel.cancel();
        }
        inbox.generation = Some(generation);
        inbox.published = Weak::new();
        inbox.acknowledged = Weak::new();
        inbox.published_stamp = None;
        inbox.reset = true;
        self.wake.notify_one();
    }
    pub fn ready(&self, navigation: NavigationId) {
        let mut inbox = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if !inbox.closed && inbox.ready.is_none_or(|old| old < navigation) {
            inbox.ready = Some(navigation);
        }
        self.wake.notify_one();
    }
    pub fn complete(&self, completion: BranchCompletion) {
        let retired = {
            let mut inbox = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if inbox.closed {
                Some(completion)
            } else {
                debug_assert!(
                    inbox.completion.is_none(),
                    "Only one branch job belongs to a pane"
                );
                inbox.completion.replace(completion)
            }
        };
        drop(retired); // This method runs on the branch worker, never under the mailbox lock.
        self.wake.notify_one();
    }
    /// A model worker blocked on output can still install its one provider result.
    /// Generation checks stay with the worker; taking a stale completion drops its permits.
    pub fn take_completion(&self) -> Option<BranchCompletion> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .completion
            .take()
    }
    pub fn pop(&self) -> Option<Input> {
        let mut inbox = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inbox.closed {
            return Some(Input::Closed);
        }
        if inbox.reset {
            inbox.reset = false;
            return inbox.generation.clone().map(Input::Reset);
        }
        if let Some(completion) = inbox.completion.take() {
            return Some(Input::Completion(completion));
        }
        if let Some((stamp, version)) = inbox.size_ack.take() {
            return Some(Input::Command(TreeCommand::Sizes(
                midnight_core::TreeSizeCommand::Acknowledge { stamp, version },
            )));
        }
        if let Some(command) = inbox.commands.pop_front() {
            return Some(Input::Command(command));
        }
        inbox.ready.take().map(Input::Ready)
    }
    /// A returned provider result yields to all already admitted input before
    /// another callback starts; no native call is interrupted or detached here.
    pub fn has_pending_input(&self) -> bool {
        let inbox = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        inbox.closed
            || inbox.reset
            || inbox.completion.is_some()
            || inbox.size_ack.is_some()
            || !inbox.commands.is_empty()
            || inbox.ready.is_some()
    }
    pub fn close(&self) {
        let mut inbox = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inbox.closed = true;
        if let Some(generation) = &inbox.generation {
            generation.cancel.cancel();
        }
        self.wake.notify_one();
    }
    pub fn drain(&self) {
        let retired = {
            let mut inbox = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            (std::mem::take(&mut inbox.commands), inbox.completion.take())
        };
        drop(retired);
    }
}

fn known_view(
    command: &TreeCommand,
    published: &Weak<TreeProjection>,
    acknowledged: &Weak<TreeProjection>,
) -> bool {
    let view = match command {
        TreeCommand::Sizes(midnight_core::TreeSizeCommand::Compute(request)) => &request.view,
        TreeCommand::Sizes(_) => return true,
        TreeCommand::RefreshLoaded(request) => &request.view,
        TreeCommand::Edit(request) => &request.view,
        TreeCommand::Rename(request) => &request.view,
        TreeCommand::Enable {
            root,
            selected: midnight_core::TreeRootSelection::Native(selection),
            ..
        } => return Arc::ptr_eq(selection.basis(), &root.index),
        TreeCommand::Enable { .. } => return true,
        TreeCommand::Commit(view)
        | TreeCommand::CaptureDropDirectory { view, .. }
        | TreeCommand::Branch { view, .. }
        | TreeCommand::Viewport { view, .. }
        | TreeCommand::Navigate { view, .. } => view,
        TreeCommand::Select(request) => {
            if matches!(&request.action, midnight_core::TreeSelectionAction::Remove(rows) if rows.len() > 64)
            {
                return false;
            }
            if matches!(&request.action, midnight_core::TreeSelectionAction::Replace(replacement)
                if !replacement.admits_view(&request.view))
            {
                return false;
            }
            if request
                .seed
                .as_ref()
                .is_some_and(|seed| !Arc::ptr_eq(seed.basis(), &request.view))
            {
                return false;
            }
            &request.view
        }
        TreeCommand::Clipboard(request) => {
            if !Arc::ptr_eq(request.selection.basis(), &request.view) {
                return false;
            }
            &request.view
        }
        TreeCommand::UserCommand(request) => {
            if !Arc::ptr_eq(request.selection.basis(), &request.view) {
                return false;
            }
            &request.view
        }
        TreeCommand::Checksums(request) => {
            if !Arc::ptr_eq(request.selection.basis(), &request.view) {
                return false;
            }
            &request.view
        }
        TreeCommand::Metadata(request) => {
            if !Arc::ptr_eq(request.selection.basis(), &request.view) {
                return false;
            }
            &request.view
        }
        TreeCommand::Delete(request) => {
            if !Arc::ptr_eq(request.selection.basis(), &request.view) {
                return false;
            }
            &request.view
        }
        TreeCommand::Transfer(request) => {
            if !Arc::ptr_eq(request.selection.basis(), &request.view) {
                return false;
            }
            &request.view
        }
        TreeCommand::Reproject(request) => {
            if !Arc::ptr_eq(request.selection.basis(), &request.view)
                || request.selection.bits().len() > TreeProjection::MAX_ROWS.div_ceil(64)
                || request.selection.bits().len() != request.view.rows.len().div_ceil(64)
            {
                return false;
            }
            &request.view
        }
    };
    let view = Arc::downgrade(view);
    Weak::ptr_eq(published, &view)
        || (matches!(
            command,
            TreeCommand::Branch { .. } | TreeCommand::Navigate { .. }
        ) && Weak::ptr_eq(acknowledged, &view))
}

#[cfg(test)]
mod tests {
    use super::*;
    use midnight_core::{TreeRevision, TreeSessionId, TreeSizeCommand, TreeViewStamp};

    #[test]
    fn size_acknowledgement_bypasses_a_saturated_command_queue() {
        let lane = Mailbox::default();
        let stamp = TreeViewStamp {
            pane: PaneId::Left,
            navigation: NavigationId(4),
            session: TreeSessionId(2),
            revision: TreeRevision(3),
        };
        lane.reset(Generation {
            navigation: stamp.navigation,
            sort: SortSpec::default(),
            cancel: Cancellation::default(),
        });
        assert!(matches!(lane.pop(), Some(Input::Reset(_))));
        for request in 1..=8 {
            lane.push(TreeCommand::Sizes(TreeSizeCommand::Cancel {
                stamp,
                request: TreeRequestId(request),
            }))
            .unwrap();
        }
        assert!(
            lane.push(TreeCommand::Sizes(TreeSizeCommand::Cancel {
                stamp,
                request: TreeRequestId(9)
            }))
            .is_err()
        );
        lane.push(TreeCommand::Sizes(TreeSizeCommand::Acknowledge {
            stamp,
            version: 7,
        }))
        .unwrap();
        assert!(matches!(
            lane.pop(),
            Some(Input::Command(TreeCommand::Sizes(
                TreeSizeCommand::Acknowledge { version: 7, .. }
            )))
        ));
        for _ in 0..8 {
            assert!(matches!(lane.pop(), Some(Input::Command(_))));
        }
    }
    #[test]
    fn replacement_selection_rejects_foreign_scope_before_mailbox_admission() {
        use midnight_core::{
            DirectoryIndex, SelectionRevision, TreeLayoutRevision, TreeRootSnapshot,
            TreeSelectionAction, TreeSelectionReplacement, TreeSelectionRequest,
            TreeSelectionSnapshot,
        };
        let stamp = TreeViewStamp {
            pane: PaneId::Left,
            navigation: NavigationId(4),
            session: TreeSessionId(2),
            revision: TreeRevision(3),
        };
        let root = TreeRootSnapshot {
            pane: stamp.pane,
            navigation: stamp.navigation,
            entries: Arc::from([]),
            index: Arc::new(DirectoryIndex::default()),
        };
        let view =
            Arc::new(TreeProjection::new(root, stamp, TreeLayoutRevision(1), vec![]).unwrap());
        let lane = Mailbox::default();
        lane.reset(Generation {
            navigation: stamp.navigation,
            sort: SortSpec::default(),
            cancel: Cancellation::default(),
        });
        lane.pop();
        assert!(lane.publish(&view));
        for changed in [true, false] {
            let mut basis = (*view).clone();
            if changed {
                basis.stamp.session = TreeSessionId(999);
            }
            let snapshot = TreeSelectionSnapshot::from_bits(Arc::new(basis), vec![]).unwrap();
            let replacement = Arc::new(TreeSelectionReplacement::from_snapshot(&snapshot).unwrap());
            let accepted = lane.push(TreeCommand::Select(TreeSelectionRequest {
                view: view.clone(),
                revision: SelectionRevision(1),
                seed: None,
                seed_revision: SelectionRevision(0),
                action: TreeSelectionAction::Replace(replacement),
            }));
            assert_eq!(accepted.is_ok(), !changed);
            assert_eq!(
                lane.pop().is_some(),
                !changed,
                "foreign replacement never occupies a queue slot"
            );
        }
    }
    #[test]
    fn viewport_observations_coalesce_and_yield_capacity_to_visible_row_actions() {
        use midnight_core::{
            DirectoryIndex, TreeBranchAction, TreeLayoutRevision, TreeRootSnapshot, TreeViewport,
        };
        for navigate in [false, true] {
            let stamp = TreeViewStamp {
                pane: PaneId::Left,
                navigation: NavigationId(4),
                session: TreeSessionId(2),
                revision: TreeRevision(3),
            };
            let root = TreeRootSnapshot {
                pane: stamp.pane,
                navigation: stamp.navigation,
                entries: Arc::from([]),
                index: Arc::new(DirectoryIndex::default()),
            };
            let view =
                Arc::new(TreeProjection::new(root, stamp, TreeLayoutRevision(1), vec![]).unwrap());
            let lane = Mailbox::default();
            lane.reset(Generation {
                navigation: stamp.navigation,
                sort: SortSpec::default(),
                cancel: Cancellation::default(),
            });
            lane.pop();
            assert!(lane.publish(&view));
            lane.push(TreeCommand::Commit(view.clone())).unwrap();
            lane.pop();
            for _ in 0..64 {
                assert!(
                    lane.push(TreeCommand::Viewport {
                        view: view.clone(),
                        rows: TreeViewport::new(&view, vec![]).unwrap()
                    })
                    .is_ok(),
                    "latest viewport must replace earlier speculative observations"
                );
            }
            for request in 1..=7 {
                lane.push(TreeCommand::Sizes(TreeSizeCommand::Cancel {
                    stamp,
                    request: TreeRequestId(request),
                }))
                .unwrap();
            }
            let action = if navigate {
                TreeCommand::Navigate {
                    view: view.clone(),
                    request: TreeRequestId(8),
                    row: TreeRowId(1),
                    navigation: NavigationId(5),
                }
            } else {
                TreeCommand::Branch {
                    view: view.clone(),
                    request: TreeRequestId(8),
                    row: TreeRowId(1),
                    action: TreeBranchAction::Expand,
                }
            };
            assert!(
                lane.push(action).is_ok(),
                "a speculative viewport cannot reject a visible row action"
            );
            assert!(
                lane.push(TreeCommand::Sizes(TreeSizeCommand::Cancel {
                    stamp,
                    request: TreeRequestId(9)
                }))
                .is_err(),
                "eight explicit intents remain the hard bound"
            );
            let mut commands = 0;
            while let Some(Input::Command(command)) = lane.pop() {
                assert!(!matches!(command, TreeCommand::Viewport { .. }));
                commands += 1;
            }
            assert_eq!(commands, 8);
        }
    }
}
