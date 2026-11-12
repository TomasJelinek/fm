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
    Rejected(TreeCommand),
    Superseded(TreeCommand),
}
impl Mailbox {
    /// Submit from the model worker; queued work retains its existing branch permit.
    pub fn start_branch(self: &Arc<Self>, job: BranchJob) -> tokio::task::JoinHandle<()> {
        let inbox = self.clone();
        tokio::task::spawn_blocking(move || inbox.complete(job.run()))
    }
    pub fn push(&self, command: TreeCommand) -> Result<(), PushError> {
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
            return Ok(());
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
        if inbox.commands.len() >= 8 || !known_view(&command, &inbox.published) {
            return Err(PushError::Rejected(command));
        }
        inbox.commands.push_back(command);
        self.wake.notify_one();
        Ok(())
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

fn known_view(command: &TreeCommand, published: &Weak<TreeProjection>) -> bool {
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
    Weak::ptr_eq(published, &Arc::downgrade(view))
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
}
