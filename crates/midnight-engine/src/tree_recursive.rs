//! Bounded recursive batches preserve the last published action model until Commit.
use super::*;
use midnight_core::{EntryKind, TreeDiagnostic, TreeEvent};
use std::time::{Duration, Instant};

const MAX_BATCH_RESULTS: usize = 128;
const MAX_BATCH_TIME: Duration = Duration::from_millis(50);

#[derive(Clone)]
struct Frame {
    branch: BranchId,
    next_child: usize,
    entered: bool,
    // Version of the ordered child sequence the numeric cursor indexes.
    children_version: Option<u64>,
}

#[derive(Clone)]
pub(super) struct Walk {
    root: BranchId,
    row: TreeRowId,
    request: TreeRequestId,
    frames: Vec<Frame>,
    active: Option<BranchId>,
    // Folders on the deepest admitted row whose own folders the depth budget left
    // out: the first one the walk met, and how many there are.
    depth_stops: Option<(BranchId, usize)>,
}

pub(super) struct Batch {
    checkpoint: crate::providers::tree::TreeCheckpoint,
    walk: Walk,
    started: Instant,
    results: usize,
    dirty: bool,
    #[cfg(test)]
    test_elapsed: Option<bool>,
}

impl Batch {
    fn time_expired(&self) -> bool {
        #[cfg(test)]
        if let Some(elapsed) = self.test_elapsed {
            return elapsed;
        }
        self.started.elapsed() >= MAX_BATCH_TIME
    }
}

enum Pause {
    Capacity,
    Changed,
    Admission,
}

/// Why a walk left folders unexpanded, for its notice.
enum Incomplete {
    Paused(Pause),
    /// The depth budget refused the listing of this many folders on its deepest row.
    Depth(usize),
}

impl Walk {
    fn pending(&mut self, tree: &DirectoryTree) -> Result<Option<BranchId>, Pause> {
        loop {
            // The next unentered child may already be on the stack. Validate
            // every ancestor cursor before dispatching it after a refresh.
            for frame in &self.frames {
                if frame.entered
                    && frame.children_version.is_some()
                    && tree
                        .listed_children(frame.branch)
                        .map(|(version, _)| version)
                        != frame.children_version
                {
                    return Err(Pause::Changed);
                }
            }
            for frame in self.frames.iter().take(self.frames.len().saturating_sub(1)) {
                if tree
                    .state(frame.branch)
                    .is_none_or(|state| !state.expanded || state.error.is_some())
                {
                    return Err(Pause::Changed);
                }
            }
            let Some(frame) = self.frames.last_mut() else {
                return Ok(None);
            };
            if tree.recursive_excluded(frame.branch, self.request.0) {
                self.frames.pop();
                continue;
            }
            let Some(state) = tree.state(frame.branch) else {
                return Err(Pause::Changed);
            };
            if !frame.entered {
                return Ok(Some(frame.branch));
            }
            if !state.expanded || state.error.is_some() {
                self.frames.pop();
                continue;
            }
            let Some((version, children)) = tree.listed_children(frame.branch) else {
                return Err(Pause::Changed);
            };
            // A refresh may reorder, insert or remove child occurrences. A numeric
            // cursor is valid only for the exact child sequence it was taken from;
            // an unchanged re-listing keeps that sequence and its version.
            if frame.children_version != Some(version) {
                return Err(Pause::Changed);
            }
            let Some(child) = children.get(frame.next_child).copied() else {
                self.frames.pop();
                continue;
            };
            frame.next_child += 1;
            let Some(state) = tree.state(child) else {
                return Err(Pause::Changed);
            };
            if !matches!(
                state.entry.kind,
                EntryKind::Directory | EntryKind::DirectoryLink
            ) && state.disclosure == Disclosure::None
            {
                continue;
            }
            // A walk from a top-level row reaches the deepest admitted row in
            // `depth + 1` frames; the tree refuses to list anything deeper.
            if self.frames.len() > TreeLimits::default().depth {
                return Err(Pause::Capacity);
            }
            self.frames.push(Frame {
                branch: child,
                next_child: 0,
                entered: false,
                children_version: None,
            });
        }
    }
}

impl Session {
    pub(super) fn start_recursive(
        &mut self,
        root: BranchId,
        row: TreeRowId,
        request: TreeRequestId,
    ) {
        self.stop_recursive();
        self.recursive_notice = None;
        self.recursive_publication = false;
        self.queue.retain(|pending| {
            pending.kind != TreeRequest::Probe || !in_subtree(&self.tree, pending.branch, root)
        });
        let mut frames = Vec::with_capacity(TreeLimits::default().depth + 1);
        frames.push(Frame {
            branch: root,
            next_child: 0,
            entered: false,
            children_version: None,
        });
        self.recursive = Some(Walk {
            root,
            row,
            request,
            frames,
            active: None,
            depth_stops: None,
        });
    }

    pub(super) fn stop_recursive(&mut self) {
        self.rollback_unpublished();
        if let Some(previous) = self.recursive.take()
            && let Some(active) = previous.active
        {
            self.tree.cancel_work(active);
            self.versions.remove(&active);
        }
    }

    /// The controller invokes this before explicit commands under a published view.
    /// Counters are deliberately outside the tree checkpoint and never roll back.
    pub(crate) fn rollback_unpublished(&mut self) -> bool {
        let Some(batch) = self.recursive_batch.take() else {
            return false;
        };
        if let Some(active) = self.recursive.as_ref().and_then(|walk| walk.active) {
            self.tree.cancel_work(active);
            self.versions.remove(&active);
        }
        self.tree.rollback_unpublished(batch.checkpoint);
        self.recursive = Some(batch.walk);
        true
    }

    pub(super) fn recursive_ready(&self) -> bool {
        !self.lifetime.is_cancelled()
            && self
                .recursive
                .as_ref()
                .is_some_and(|walk| walk.active.is_none())
            && self.projection_accepted()
    }

    pub(super) fn explicit_work_ready(&self) -> bool {
        self.recursive_ready()
            || self
                .queue
                .iter()
                .any(|pending| pending.kind != TreeRequest::Probe)
    }

    pub(crate) fn recursive_batch_active(&self) -> bool {
        self.recursive_batch.is_some()
    }

    pub(super) fn exclude_recursive(&mut self, branch: BranchId) {
        self.rollback_unpublished();
        let Some(walk) = &mut self.recursive else {
            return;
        };
        if in_subtree(&self.tree, walk.root, branch) {
            self.stop_recursive();
            return;
        }
        self.tree.exclude_recursive(branch, walk.request.0);
        walk.frames
            .retain(|frame| !in_subtree(&self.tree, frame.branch, branch));
        if walk
            .active
            .is_some_and(|active| in_subtree(&self.tree, active, branch))
        {
            walk.active = None;
        }
    }

    pub(super) fn take_recursive_branch(&mut self) -> Result<Option<PendingBranch>, ProviderError> {
        if self.recursive_batch.is_none() {
            let Some(walk) = self.recursive.clone() else {
                return Ok(None);
            };
            // A batch only lists descendants of its walk root; nothing else runs
            // until the batch is published or rolled back.
            self.recursive_batch = Some(Batch {
                checkpoint: self.tree.checkpoint(walk.root),
                walk,
                started: Instant::now(),
                results: 0,
                dirty: false,
                #[cfg(test)]
                test_elapsed: None,
            });
        }
        let next = self
            .recursive
            .as_mut()
            .map(|walk| walk.pending(&self.tree))
            .transpose();
        let branch = match next {
            Ok(Some(Some(branch))) => branch,
            Ok(_) => {
                let walk = self.recursive.take();
                let cancel = self.lifetime.clone();
                self.complete_recursive(walk, &cancel)?;
                return Ok(None);
            }
            Err(reason) => {
                self.pause_recursive_reason(reason)?;
                return Ok(None);
            }
        };
        let version = self.next_work;
        let Some(next) = version.checked_add(1) else {
            self.pause_recursive_reason(Pause::Capacity)?;
            return Ok(None);
        };
        let Some(walk) = &mut self.recursive else {
            return Ok(None);
        };
        let Some(frame) = walk.frames.last_mut() else {
            return Err(ProviderError::StaleReference);
        };
        frame.entered = true;
        walk.active = Some(branch);
        self.next_work = next;
        self.versions.insert(branch, version);
        // The walk lists this branch itself. A viewport probe queued while it was
        // still unknown would otherwise later target an expanded node.
        self.queue
            .retain(|pending| pending.branch != branch || pending.kind != TreeRequest::Probe);
        Ok(Some(PendingBranch {
            branch,
            origin: WorkOrigin::Recursive {
                walk: walk.request,
                #[cfg(test)]
                row: walk.row,
            },
            kind: TreeRequest::Expand,
            version,
            request: walk.request,
        }))
    }

    pub(super) fn apply_recursive(
        &mut self,
        completion: BranchCompletion,
        cancel: &Cancellation,
    ) -> Result<bool, ProviderError> {
        let BranchCompletion {
            stamp,
            pending,
            result,
            _registration,
            _permit,
        } = completion;
        let WorkOrigin::Recursive { walk: request, .. } = pending.origin else {
            return Ok(false);
        };
        let branch = pending.branch;
        let current = self.projection.stamp;
        let Some(walk) = &mut self.recursive else {
            return Ok(false);
        };
        if walk.request != request {
            return Ok(false);
        }
        if walk.active == Some(branch) {
            walk.active = None;
        }
        if stamp.pane != current.pane
            || stamp.navigation != current.navigation
            || stamp.session != current.session
            || self.versions.get(&branch) != Some(&pending.version)
            || self.recursive_batch.is_none()
        {
            return Ok(false);
        }
        self.versions.remove(&branch);
        if cancel.is_cancelled() {
            self.stop_recursive();
            return Ok(false);
        }
        let applied = self.tree.apply(result);
        // The native callback has really returned. Release its I/O ownership
        // before potentially expensive projection construction or output waiting.
        drop(_permit);
        drop(_registration);
        if !applied {
            return Ok(false);
        }
        // The model changed, a failure of this row included: the batch publishes it.
        let Some(batch) = &mut self.recursive_batch else {
            return Ok(false);
        };
        batch.results += 1;
        batch.dirty = true;
        if self.tree.depth_limited(branch) {
            // Only this folder's listing is refused: its folders are one level too
            // deep. Like a denied folder, it stays local to its row; the walk goes
            // on with the others and says at its end what it left out.
            if let Some(walk) = &mut self.recursive {
                let stops = walk.depth_stops.get_or_insert((branch, 0));
                stops.1 = stops.1.saturating_add(1);
            }
        } else if self
            .tree
            .state(branch)
            .is_some_and(|row| row.error == Some(ProviderError::ResourceLimited))
        {
            self.pause_recursive_reason(Pause::Capacity)?;
            return Ok(false);
        }
        let mut finished = None;
        if let Some(walk) = &mut self.recursive {
            if let Some(frame) = walk
                .frames
                .last_mut()
                .filter(|frame| frame.branch == branch)
            {
                frame.children_version = self
                    .tree
                    .listed_children(branch)
                    .map(|(version, _)| version);
            }
            match walk.pending(&self.tree) {
                Ok(Some(_)) => {}
                Ok(None) => finished = self.recursive.take(),
                Err(reason) => {
                    self.pause_recursive_reason(reason)?;
                    return Ok(false);
                }
            }
        }
        let Some(batch) = &self.recursive_batch else {
            return Ok(false);
        };
        // Check only after a real callback return; 50 ms never invents cancellation
        // or releases a still-running native worker permit.
        let yields = batch.results >= MAX_BATCH_RESULTS || batch.time_expired();
        if finished.is_some() {
            return self.complete_recursive(finished, cancel);
        }
        if self.recursive.is_none() || yields {
            return self.finish_recursive_batch(cancel);
        }
        Ok(false)
    }

    /// The walk reached everything it could: publish its last batch, then say
    /// which folders the depth budget left out.
    fn complete_recursive(
        &mut self,
        walk: Option<Walk>,
        cancel: &Cancellation,
    ) -> Result<bool, ProviderError> {
        let published = self.finish_recursive_batch(cancel)?;
        // A projection that cannot hold the batch rolls it back and reports that.
        if self.recursive_notice.is_some() {
            return Ok(published);
        }
        if let Some(walk) = walk
            && let Some((branch, folders)) = walk.depth_stops
        {
            self.recursive_notice =
                Some(self.incomplete_notice(&walk, Some(branch), Incomplete::Depth(folders)));
        }
        Ok(published)
    }

    fn finish_recursive_batch(&mut self, cancel: &Cancellation) -> Result<bool, ProviderError> {
        let Some(batch) = self.recursive_batch.take() else {
            return Ok(false);
        };
        if !batch.dirty {
            return Ok(false);
        }
        match self.update_projection(cancel) {
            Ok(()) => {
                self.recursive_publication = true;
                Ok(true)
            }
            Err(error) => {
                self.tree.rollback_unpublished(batch.checkpoint);
                self.recursive = Some(batch.walk);
                if error == ProviderError::ResourceLimited {
                    self.pause_recursive_reason(Pause::Capacity)?;
                    Ok(false)
                } else {
                    // Malformed provider responses retain their existing failure
                    // classification; they are not converted into capacity pauses.
                    Err(error)
                }
            }
        }
    }

    pub(crate) fn pause_recursive(&mut self, error: &ProviderError) -> Result<bool, ProviderError> {
        let reason = if *error == ProviderError::ResourceLimited {
            Pause::Capacity
        } else {
            Pause::Admission
        };
        self.pause_recursive_reason(reason)
    }

    fn pause_recursive_reason(&mut self, reason: Pause) -> Result<bool, ProviderError> {
        // The walk stops at the folder it was listing or about to list.
        let stop = self
            .recursive
            .as_ref()
            .and_then(|walk| walk.frames.last())
            .map(|frame| frame.branch);
        // What the batch listed within the budgets stays: publish it. A projection
        // that cannot hold it rolls the batch back and reports that stop instead.
        let cancel = self.lifetime.clone();
        self.finish_recursive_batch(&cancel)?;
        let Some(walk) = self.recursive.take() else {
            return Ok(false);
        };
        if let Some(active) = walk.active {
            self.tree.cancel_work(active);
            self.versions.remove(&active);
        }
        self.recursive_notice =
            Some(self.incomplete_notice(&walk, stop, Incomplete::Paused(reason)));
        Ok(true)
    }

    /// Say where the walk stopped and why, against the published view: the row of
    /// the folder where it stopped when that view shows it, else the walk's own row.
    fn incomplete_notice(&self, walk: &Walk, stop: Option<BranchId>, why: Incomplete) -> TreeEvent {
        let shown = stop.and_then(|branch| {
            let row = self.projection_state.row(branch)?;
            self.projection
                .index
                .row_positions
                .contains_key(&row)
                .then_some((branch, row))
        });
        let (branch, row) = shown.unwrap_or((walk.root, walk.row));
        let name = self
            .tree
            .state(branch)
            .map(|state| state.entry.display_name.clone())
            .unwrap_or_default();
        let message = match why {
            Incomplete::Depth(folders) => {
                let others = match folders.saturating_sub(1) {
                    0 => String::new(),
                    1 => " and 1 other folder".to_owned(),
                    others => format!(" and {others} other folders"),
                };
                format!(
                    "Recursive expansion is incomplete: the tree shows at most {} levels of \
                     subfolders, so the folders inside “{name}”{others} are not shown.",
                    self.tree.depth_limit()
                )
            }
            Incomplete::Paused(pause) => {
                let reason = match pause {
                    Pause::Capacity => "the tree resource budget is exhausted",
                    Pause::Changed => "a loaded branch changed. Restart expansion to continue",
                    Pause::Admission => "the branch could not be admitted",
                };
                format!(
                    "Recursive expansion is incomplete: it stopped at “{name}” because {reason}."
                )
            }
        };
        TreeEvent::Rejected {
            stamp: self.projection.stamp,
            request: walk.request,
            row,
            error: TreeDiagnostic::new(&message),
        }
    }

    pub(crate) fn take_recursive_notice(&mut self) -> Option<TreeEvent> {
        self.recursive_notice.take()
    }

    pub(crate) fn take_recursive_publication(&mut self) -> Option<TreePublication> {
        if !std::mem::take(&mut self.recursive_publication) {
            return None;
        }
        self.publication().ok()
    }

    #[cfg(test)]
    pub(crate) fn set_recursive_batch_elapsed(&mut self, elapsed: bool) -> bool {
        let Some(batch) = &mut self.recursive_batch else {
            return false;
        };
        batch.test_elapsed = Some(elapsed);
        true
    }

    #[cfg(test)]
    pub(crate) fn expire_recursive_batch_deadline(&mut self) -> bool {
        let Some(batch) = &mut self.recursive_batch else {
            return false;
        };
        let Some(started) = Instant::now().checked_sub(MAX_BATCH_TIME) else {
            return false;
        };
        batch.started = started;
        batch.test_elapsed = None;
        true
    }
}
