//! Bounded, revocable Find authority with runtime-independent final retirement.
use super::*;
use crate::retirement::WorkerOwned;
use std::fmt;

struct Capture {
    request: FindValidationRequest,
    owned: WorkerOwned<Payload>,
}
struct Payload {
    result: Arc<FindResult>,
    state: Arc<Mutex<State>>,
    service: Arc<dyn FindService>,
    lifetime: Cancellation,
    _scope: TaskLifetime,
    _slot: tokio::sync::OwnedSemaphorePermit,
}
impl fmt::Debug for Capture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CapturedFindFile")
            .field("request", &self.request)
            .finish_non_exhaustive()
    }
}
impl CapturedFindFile for Capture {
    fn request(&self) -> FindValidationRequest {
        self.request
    }
    fn entry(&self) -> &FindEntry {
        &self.owned.result.entry
    }
    fn validate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.check(cancel)?;
        find_worker::validate_result(self.owned.service.as_ref(), self.entry(), cancel)?;
        self.check(cancel)
    }
}
impl Capture {
    fn check(&self, cancel: &Cancellation) -> Result<(), FsError> {
        let state = self
            .owned
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if cancel.is_cancelled()
            || self.owned.lifetime.is_cancelled()
            || state.owner != Some((self.request.session, self.request.generation))
        {
            return Err(find_error(
                &self.entry().path,
                io::ErrorKind::Interrupted,
                "The Find result action was cancelled or its search was replaced/closed.",
            ));
        }
        Ok(())
    }
}

pub(super) fn capture(
    services: &Services,
    validation: &Validation,
    cancel: &Cancellation,
) -> Result<Arc<dyn CapturedFindFile>, FsError> {
    let entry = &validation.result.entry;
    if entry.kind != EntryKind::File {
        return Err(find_error(
            &entry.path,
            io::ErrorKind::InvalidInput,
            "View, Edit and Open require a regular file. Use Go to file for folders.",
        ));
    }
    let slot = services
        .capture_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            find_error(
                &entry.path,
                io::ErrorKind::WouldBlock,
                "Sixteen Find file actions are retained. Close a viewer and try again.",
            )
        })?;
    let lease = services.captures.try_reserve().map_err(|_| {
        find_error(
            &entry.path,
            io::ErrorKind::WouldBlock,
            "Find action cleanup capacity is busy. Try again.",
        )
    })?;
    let service = services.native.clone().ok_or_else(|| {
        find_error(
            &entry.path,
            io::ErrorKind::Unsupported,
            "Native Find is unavailable.",
        )
    })?;
    Ok(Arc::new(Capture {
        request: validation.request,
        owned: lease.own(Payload {
            result: validation.result.clone(),
            state: validation.state.clone(),
            service,
            lifetime: cancel.clone(),
            _scope: services.scope.clone(),
            _slot: slot,
        }),
    }))
}
