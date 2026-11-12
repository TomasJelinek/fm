//! Focused native Rename capture and runtime-independent ownership.
use crate::{
    providers::{ProviderError, ProviderTarget},
    retirement::{RetirementDomain, WorkerOwned},
    tree_actor::{CapturedTarget, Session},
};
use midnight_core::*;
use std::{
    io,
    path::{Component, Path},
    sync::Arc,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub(crate) const RETIREMENT_CAPACITY: usize = crate::tree_controller::CAPTURE_CAPACITY;
#[derive(Clone)]
pub(crate) struct Retirement {
    pub(crate) domain: RetirementDomain,
}
#[cfg(test)]
impl Retirement {
    pub(crate) fn testing() -> Self {
        Self {
            domain: RetirementDomain::try_start(RETIREMENT_CAPACITY, 0).unwrap(),
        }
    }
}
struct Payload {
    captured: CapturedTarget,
    // Return capture admission only after the native target and registration are destroyed.
    _permit: OwnedSemaphorePermit,
}
struct Source {
    owner: WorkerOwned<Payload>,
    stamp: TreeViewStamp,
    row: TreeRowId,
    request: TreeRequestId,
}
impl std::fmt::Debug for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TreeRenameTarget")
            .field("stamp", &self.stamp)
            .field("row", &self.row)
            .field("request", &self.request)
            .finish_non_exhaustive()
    }
}
impl Session {
    pub(crate) fn capture_rename(
        &mut self,
        request: &TreeRenameRequest,
        capacity: &Arc<Semaphore>,
        retirement: &Retirement,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn CapturedRenameTarget>, ProviderError> {
        if cancel.is_cancelled() || request.cancel.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        if request.request.0 == 0 || request.request.0 <= self.last_request.0 {
            return Err(ProviderError::StaleReference);
        }
        let permit = capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| ProviderError::ResourceLimited)?;
        let lease = retirement
            .domain
            .try_reserve()
            .map_err(|_| ProviderError::ResourceLimited)?;
        let linked = cancel.linked(&request.cancel);
        let captured = self.capture_target(&request.view, request.row, &linked)?;
        let ProviderTarget::Native(path) = captured.target.value()? else {
            return Err(ProviderError::Unsupported);
        };
        validate_path(path)?;
        if linked.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        self.last_request = request.request;
        Ok(Arc::new(Source {
            owner: lease.own(Payload {
                captured,
                _permit: permit,
            }),
            stamp: request.view.stamp,
            row: request.row,
            request: request.request,
        }))
    }
}
impl CapturedRenameTarget for Source {
    fn stamp(&self) -> TreeViewStamp {
        self.stamp
    }
    fn request(&self) -> TreeRequestId {
        self.request
    }
    fn row(&self) -> TreeRowId {
        self.row
    }
    fn prepare(
        &self,
        service: &dyn MutationService,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn PreparedMutation>, FsError> {
        check_cancel(cancel)?;
        let ProviderTarget::Native(path) = self.owner.captured.target.value().map_err(failure)?
        else {
            return Err(failure(ProviderError::Unsupported));
        };
        validate_path(path).map_err(failure)?;
        let prepared = service.prepare(path, cancel)?;
        if prepared.item().path != *path {
            return Err(failure(ProviderError::InvalidResponse(
                "Rename target changed during preparation.",
            )));
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
            "Rename requires a native leaf, not parent navigation.",
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
        "prepare tree Rename",
        Path::new(""),
        io::Error::new(kind, error),
    )
}
