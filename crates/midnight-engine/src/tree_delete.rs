//! Retained native Delete roots; preparation and final ownership release run on workers.
use crate::{
    providers::{ProviderError, ProviderTarget},
    tree_actor::{CapturedSelection, Session},
};
use midnight_core::*;
use std::{collections::HashSet, io, path::Path, sync::Arc};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

struct Sources {
    capture: CapturedSelection,
    covering: HashSet<TreeRowId>,
    mode: DeleteMode,
    request: TreeRequestId,
    _permit: OwnedSemaphorePermit,
}
impl std::fmt::Debug for Sources {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TreeDeleteSources")
            .field("roots", &self.capture.targets.len())
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}
impl Session {
    pub(crate) fn capture_delete(
        &mut self,
        request: &TreeDeleteRequest,
        capacity: &Arc<Semaphore>,
        cancel: &Cancellation,
    ) -> Result<TreeDeleteCapture, ProviderError> {
        if request.request.0 == 0
            || request.request.0 <= self.last_request.0
            || request.cancel.is_cancelled()
            || cancel.is_cancelled()
        {
            return Err(ProviderError::StaleReference);
        }
        let permit = capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| ProviderError::ResourceLimited)?;
        let mut capture = self.capture_selection(
            &request.view,
            request.selection_revision,
            &request.selection,
            request.fallback,
            &request.cancel,
        )?;
        // Validate the whole original selection before suppressing descendants:
        // a native parent must never hide a selected unsupported provider target.
        for target in &capture.targets {
            if !matches!(target.captured.target.value()?, ProviderTarget::Native(_)) {
                return Err(ProviderError::Unsupported);
            }
        }
        let selected: HashSet<_> = capture.targets.iter().map(|target| target.row).collect();
        let mut covering = HashSet::new();
        let mut keep = Vec::with_capacity(capture.targets.len());
        for target in &capture.targets {
            let ancestor = covering_ancestor(&capture.view, target.row, &selected)?;
            if let Some(ancestor) = ancestor {
                covering.insert(ancestor);
            }
            keep.push(ancestor.is_none());
        }
        let mut positions = keep.into_iter();
        capture
            .targets
            .retain(|_| positions.next().unwrap_or(false));
        if cancel.is_cancelled() || request.cancel.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        self.last_request = request.request;
        Ok(TreeDeleteCapture {
            mode: request.mode,
            root_count: capture.targets.len(),
            sources: Arc::new(Sources {
                capture,
                covering,
                mode: request.mode,
                request: request.request,
                _permit: permit,
            }),
        })
    }
}

fn covering_ancestor(
    view: &TreeProjection,
    row: TreeRowId,
    selected: &HashSet<TreeRowId>,
) -> Result<Option<TreeRowId>, ProviderError> {
    let position = view
        .index
        .row_positions
        .get(&row)
        .ok_or(ProviderError::StaleReference)?;
    let parent = view
        .rows
        .get(*position)
        .ok_or(ProviderError::StaleReference)?
        .parent;
    covering_chain(parent, selected, |id| {
        let position = view
            .index
            .row_positions
            .get(&id)
            .ok_or(ProviderError::StaleReference)?;
        let row = view
            .rows
            .get(*position)
            .ok_or(ProviderError::StaleReference)?;
        Ok((row.entry.kind, row.parent))
    })
}

fn covering_chain(
    mut parent: Option<TreeRowId>,
    selected: &HashSet<TreeRowId>,
    mut lookup: impl FnMut(TreeRowId) -> Result<(EntryKind, Option<TreeRowId>), ProviderError>,
) -> Result<Option<TreeRowId>, ProviderError> {
    let mut crossed_boundary = false;
    for _ in 0..=TreeProjection::MAX_DEPTH {
        let Some(id) = parent else { return Ok(None) };
        let (kind, next) = lookup(id)?;
        crossed_boundary |= kind != EntryKind::Directory;
        if selected.contains(&id) {
            if crossed_boundary {
                return Err(ProviderError::Failed(
                    "Selected targets overlap through a link or non-directory. Delete them separately."
                        .into(),
                ));
            }
            return Ok(Some(id));
        }
        parent = next;
    }
    Err(ProviderError::StaleReference)
}

#[cfg(test)]
#[path = "tree_delete_ancestry_tests.rs"]
mod ancestry_tests;

impl CapturedDeleteSources for Sources {
    fn stamp(&self) -> TreeViewStamp {
        self.capture.view.stamp
    }
    fn request(&self) -> TreeRequestId {
        self.request
    }
    fn mode(&self) -> DeleteMode {
        self.mode
    }
    fn root_count(&self) -> usize {
        self.capture.targets.len()
    }
    fn prepare(
        &self,
        service: &dyn MutationService,
        cancel: &Cancellation,
    ) -> Result<Vec<Arc<dyn PreparedMutation>>, FsError> {
        // Revalidate all retained provider contexts before opening any native pins.
        let mut paths = Vec::with_capacity(self.capture.targets.len());
        for target in &self.capture.targets {
            check_cancel(cancel)?;
            match target
                .captured
                .target
                .value()
                .map_err(|error| failure(&error.to_string()))?
            {
                ProviderTarget::Native(path) => paths.push(path),
                ProviderTarget::Archive(_) => {
                    return Err(failure("Tree Delete requires native targets."));
                }
            }
        }
        let mut targets = Vec::with_capacity(paths.len());
        for (occurrence, path) in self.capture.targets.iter().zip(paths) {
            check_cancel(cancel)?;
            let prepared = service.prepare(path, cancel)?;
            if prepared.item().path != *path {
                return Err(failure(
                    "Provider returned a different native Delete target.",
                ));
            }
            if self.covering.contains(&occurrence.row)
                && prepared.item().kind != EntryKind::Directory
            {
                return Err(failure(
                    "A selected covering folder changed. Select the targets again.",
                ));
            }
            targets.push(prepared);
        }
        check_cancel(cancel)?;
        Ok(targets)
    }
}
fn check_cancel(cancel: &Cancellation) -> Result<(), FsError> {
    if cancel.wait_if_paused() {
        Ok(())
    } else {
        Err(FsError::from_io(
            "prepare tree Delete",
            Path::new(""),
            io::ErrorKind::Interrupted.into(),
        ))
    }
}
fn failure(message: &str) -> FsError {
    FsError::from_io(
        "prepare tree Delete",
        Path::new(""),
        io::Error::new(io::ErrorKind::InvalidData, message),
    )
}
