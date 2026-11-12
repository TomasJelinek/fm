//! Checksum admission resolves occurrences on the model worker and retains their providers.
use super::{CapturedSelection, Session};
use crate::providers::{ProviderError, ProviderTarget};
use midnight_core::*;
use std::{io, path::Path, sync::Arc};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

struct Sources {
    capture: CapturedSelection,
    request: TreeRequestId,
    cancel: Cancellation,
    _permit: OwnedSemaphorePermit,
}
impl std::fmt::Debug for Sources {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TreeChecksumFiles")
            .field("stamp", &self.capture.view.stamp)
            .field("request", &self.request)
            .finish_non_exhaustive()
    }
}

impl Session {
    pub(crate) fn capture_checksums(
        &mut self,
        request: &TreeChecksumRequest,
        capacity: &Arc<Semaphore>,
        cancel: &Cancellation,
    ) -> Result<TreeChecksumCapture, ProviderError> {
        let linked = cancel.linked(&request.cancel);
        if linked.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        if request.request.0 == 0 || request.request.0 <= self.last_request.0 {
            return Err(ProviderError::StaleReference);
        }
        let permit = capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| ProviderError::ResourceLimited)?;
        let capture = self.capture_selection(
            &request.view,
            request.selection_revision,
            &request.selection,
            request.fallback,
            &linked,
        )?;
        // A folder must be explicitly selected, matching ordinary checksum fallback.
        for occurrence in &capture.targets {
            let row = &capture.view.rows[capture.view.index.row_positions[&occurrence.row]];
            if !matches!(row.entry.kind, EntryKind::File | EntryKind::Directory)
                || (request.selection.is_empty() && row.entry.kind != EntryKind::File)
                || !matches!(
                    occurrence.captured.target.value()?,
                    ProviderTarget::Native(_)
                )
            {
                return Err(ProviderError::Unsupported);
            }
        }
        let root = self.tree.root_target(&linked)?;
        let ProviderTarget::Native(directory) = root.value()? else {
            return Err(ProviderError::Unsupported);
        };
        let directory = directory.clone();
        let sources = Sources {
            capture,
            request: request.request,
            cancel: request.cancel.clone(),
            _permit: permit,
        };
        // Recheck every retained context after the last provider resolution.
        let targets = sources.resolve(&linked).map_err(|error| {
            if error.kind == io::ErrorKind::Interrupted {
                ProviderError::Cancelled
            } else {
                ProviderError::InvalidResponse("Checksum sources are unavailable or invalid.")
            }
        })?;
        let fallback = request
            .selection
            .is_empty()
            .then(|| targets[0].path.clone());
        self.last_request = request.request;
        Ok(TreeChecksumCapture {
            sources: Arc::new(sources),
            directory,
            fallback,
        })
    }
}

impl CapturedChecksumFiles for Sources {
    fn stamp(&self) -> TreeViewStamp {
        self.capture.view.stamp
    }
    fn request(&self) -> TreeRequestId {
        self.request
    }
    fn resolve(&self, cancel: &Cancellation) -> Result<Vec<ChecksumTarget>, FsError> {
        let check = || {
            if cancel.is_cancelled() || self.cancel.is_cancelled() {
                Err(failure(ProviderError::Cancelled))
            } else {
                Ok(())
            }
        };
        check()?;
        let mut targets = Vec::with_capacity(self.capture.targets.len());
        for occurrence in &self.capture.targets {
            check()?;
            let ProviderTarget::Native(path) =
                occurrence.captured.target.value().map_err(failure)?
            else {
                return Err(failure(ProviderError::Unsupported));
            };
            let row =
                &self.capture.view.rows[self.capture.view.index.row_positions[&occurrence.row]];
            // Recalculation reads current content, as in flat panels. Preserve kind
            // to reject replacement by a link/directory, not stale listing size/time.
            targets.push(ChecksumTarget {
                kind: Some(row.entry.kind),
                ..path.clone().into()
            });
        }
        let mut validation = ChecksumRequest::new(ChecksumJobId(1), ChecksumGeneration(1), vec![]);
        validation.targets = targets;
        validation.validate().map_err(|_| {
            failure(ProviderError::InvalidResponse(
                "Invalid native checksum targets.",
            ))
        })?;
        check()?;
        Ok(validation.targets)
    }
}

fn failure(error: ProviderError) -> FsError {
    let kind = match error {
        ProviderError::Cancelled => io::ErrorKind::Interrupted,
        ProviderError::Unsupported => io::ErrorKind::Unsupported,
        _ => io::ErrorKind::InvalidData,
    };
    FsError::from_io(
        "checksum capture",
        Path::new(""),
        io::Error::new(kind, error),
    )
}
