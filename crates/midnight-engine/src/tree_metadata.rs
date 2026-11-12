//! Selected native metadata occurrences share the existing bounded retirement domain.
use crate::{
    providers::{ProviderError, ProviderTarget},
    retirement::WorkerOwned,
    tree_actor::{CapturedSelection, Session},
    tree_rename::Retirement,
};
use midnight_core::*;
use std::{
    cmp::Reverse,
    collections::HashSet,
    io,
    path::{Component, Path},
    sync::Arc,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

struct Payload {
    capture: CapturedSelection,
    covering: HashSet<TreeRowId>,
    // Native targets and their registrations close before capture admission is returned.
    _permit: OwnedSemaphorePermit,
}
struct Sources {
    owner: WorkerOwned<Payload>,
    request: TreeRequestId,
    covered: Vec<bool>,
}
impl std::fmt::Debug for Sources {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TreeMetadataSources")
            .field("stamp", &self.stamp())
            .field("request", &self.request)
            .field("roots", &self.root_count())
            .finish_non_exhaustive()
    }
}
impl Session {
    pub(crate) fn capture_metadata(
        &mut self,
        request: &TreeMetadataRequest,
        capacity: &Arc<Semaphore>,
        retirement: &Retirement,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn CapturedMetadataSources>, ProviderError> {
        let linked = cancel.linked(&request.cancel);
        if linked.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        if request.request.0 == 0 || request.request.0 <= self.last_request.0 {
            return Err(ProviderError::StaleReference);
        }
        if request.selection.len() > MAX_METADATA_TARGETS {
            return Err(ProviderError::Failed(
                "Metadata selection exceeds the 256 pinned-target budget.".into(),
            ));
        }
        let permit = capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| ProviderError::ResourceLimited)?;
        let lease = retirement
            .domain
            .try_reserve()
            .map_err(|_| ProviderError::ResourceLimited)?;
        let mut capture = self
            .capture_selection(
                &request.view,
                request.selection_revision,
                &request.selection,
                request.fallback,
                &linked,
            )
            .map_err(target_failure)?;
        // Validate every original target before any overlap optimization or native pin.
        for target in &capture.targets {
            let ProviderTarget::Native(path) =
                target.captured.target.value().map_err(target_failure)?
            else {
                return Err(ProviderError::Unsupported);
            };
            validate_path(path)?;
        }
        let selected: HashSet<_> = capture.targets.iter().map(|target| target.row).collect();
        let mut covering = HashSet::new();
        let mut covered_rows = HashSet::new();
        for target in &capture.targets {
            if let Some(ancestor) = covering_ancestor(&capture.view, target.row, &selected)? {
                covering.insert(ancestor);
                covered_rows.insert(target.row);
            }
        }
        // Case changes to a selected parent must follow edits to selected descendants.
        capture.targets.sort_by_key(|target| {
            Reverse(capture.view.rows[capture.view.index.row_positions[&target.row]].depth)
        });
        let covered = capture
            .targets
            .iter()
            .map(|target| covered_rows.contains(&target.row))
            .collect();
        if linked.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        self.last_request = request.request;
        Ok(Arc::new(Sources {
            owner: lease.own(Payload {
                capture,
                covering,
                _permit: permit,
            }),
            request: request.request,
            covered,
        }))
    }
}
// Only our capture pool and retirement reservations are automatically retryable.
// Provider/target byte limits need an explicit retry or smaller user selection.
fn target_failure(error: ProviderError) -> ProviderError {
    if matches!(error, ProviderError::ResourceLimited) {
        ProviderError::Failed(
            "Metadata target capture exceeded a provider or resource budget. Select fewer targets or retry explicitly.".into(),
        )
    } else {
        error
    }
}
fn covering_ancestor(
    view: &TreeProjection,
    row: TreeRowId,
    selected: &HashSet<TreeRowId>,
) -> Result<Option<TreeRowId>, ProviderError> {
    let mut row = row;
    let mut crossed_boundary = false;
    for _ in 0..=TreeProjection::MAX_DEPTH {
        let position = view
            .index
            .row_positions
            .get(&row)
            .ok_or(ProviderError::StaleReference)?;
        let Some(parent) = view.rows[*position].parent else {
            return Ok(None);
        };
        let position = view
            .index
            .row_positions
            .get(&parent)
            .ok_or(ProviderError::StaleReference)?;
        crossed_boundary |= view.rows[*position].entry.kind != EntryKind::Directory;
        if selected.contains(&parent) {
            if crossed_boundary {
                return Err(ProviderError::Failed(
                    "Selected metadata roots overlap through a link or non-directory. Edit them separately.".into(),
                ));
            }
            return Ok(Some(parent));
        }
        row = parent;
    }
    Err(ProviderError::StaleReference)
}
impl CapturedMetadataSources for Sources {
    fn stamp(&self) -> TreeViewStamp {
        self.owner.capture.view.stamp
    }
    fn request(&self) -> TreeRequestId {
        self.request
    }
    fn root_count(&self) -> usize {
        self.owner.capture.targets.len()
    }
    fn covered_roots(&self) -> &[bool] {
        &self.covered
    }
    fn prepare(
        &self,
        service: &dyn MutationService,
        cancel: &Cancellation,
    ) -> Result<Vec<Arc<dyn PreparedMutation>>, FsError> {
        // Validate retained provider contexts before opening any native handles.
        let mut paths = Vec::with_capacity(self.root_count());
        for target in &self.owner.capture.targets {
            check_cancel(cancel)?;
            let ProviderTarget::Native(path) = target.captured.target.value().map_err(failure)?
            else {
                return Err(failure(ProviderError::Unsupported));
            };
            validate_path(path).map_err(failure)?;
            paths.push(path);
        }
        let mut prepared = Vec::with_capacity(paths.len());
        for (target, path) in self.owner.capture.targets.iter().zip(paths) {
            check_cancel(cancel)?;
            let pin = service.prepare(path, cancel)?;
            if pin.item().path != *path
                || (self.owner.covering.contains(&target.row)
                    && pin.item().kind != EntryKind::Directory)
            {
                return Err(failure(ProviderError::InvalidResponse(
                    "The captured metadata target or covering directory changed.",
                )));
            }
            prepared.push(pin);
        }
        check_cancel(cancel)?;
        Ok(prepared)
    }
}
fn validate_path(path: &Path) -> Result<(), ProviderError> {
    if !path.is_absolute()
        || path.as_os_str().as_encoded_bytes().contains(&0)
        || !matches!(path.components().next_back(), Some(Component::Normal(_)))
        || path.file_name().is_none()
    {
        return Err(ProviderError::InvalidResponse(
            "Metadata requires a native leaf, not parent navigation.",
        ));
    }
    Ok(())
}
fn check_cancel(cancel: &Cancellation) -> Result<(), FsError> {
    if cancel.wait_if_paused() {
        Ok(())
    } else {
        Err(failure(ProviderError::Cancelled))
    }
}
fn failure(error: ProviderError) -> FsError {
    let kind = match error {
        ProviderError::Cancelled => io::ErrorKind::Interrupted,
        ProviderError::Unsupported => io::ErrorKind::Unsupported,
        _ => io::ErrorKind::InvalidData,
    };
    FsError::from_io(
        "prepare tree metadata",
        Path::new(""),
        io::Error::new(kind, error),
    )
}
