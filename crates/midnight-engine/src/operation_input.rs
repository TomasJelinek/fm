//! A captured Delete has its own authority; flat requests retain their directory proof.
use super::*;

pub(super) enum Input {
    Flat(PrepareOperation),
    Rename(PrepareCapturedRename, RenameScope),
    Delete(PrepareCapturedDelete, DeleteScope),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DeleteScope {
    pub(super) stamp: TreeViewStamp,
    pub(super) request: TreeRequestId,
    pub(super) mode: DeleteMode,
    pub(super) count: usize,
}
impl DeleteScope {
    pub(super) fn capture(sources: &dyn CapturedDeleteSources) -> Self {
        Self {
            stamp: sources.stamp(),
            request: sources.request(),
            mode: sources.mode(),
            count: sources.root_count(),
        }
    }
    pub(super) fn valid(self) -> bool {
        self.stamp.session.0 != 0
            && self.stamp.revision.0 != 0
            && self.request.0 != 0
            && (1..=MAX_CAPTURED_DELETE_ROOTS).contains(&self.count)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RenameScope {
    pub(super) stamp: TreeViewStamp,
    pub(super) request: TreeRequestId,
    pub(super) row: TreeRowId,
}
impl RenameScope {
    pub(super) fn capture(source: &dyn CapturedRenameTarget) -> Self {
        Self {
            stamp: source.stamp(),
            request: source.request(),
            row: source.row(),
        }
    }
    pub(super) fn valid(self) -> bool {
        self.stamp.session.0 != 0
            && self.stamp.revision.0 != 0
            && self.request.0 != 0
            && self.row.0 != 0
    }
}
impl Input {
    pub(super) fn initial(&self) -> OperationProgress {
        match self {
            Self::Flat(request) => worker::initial(request),
            Self::Rename(request, _) => worker::initial_for(
                request.id,
                MutationKind::Rename,
                request.source_directory.clone(),
            ),
            Self::Delete(request, scope) => worker::initial_for(
                request.id,
                MutationKind::Delete(scope.mode),
                request.source_directory.clone(),
            ),
        }
    }
    pub(super) fn prepare(
        self,
        service: Option<Arc<dyn MutationService>>,
        cancel: Cancellation,
        status: Status,
        metadata_budget: Arc<Semaphore>,
    ) -> Result<worker::Job, FsError> {
        match self {
            Self::Flat(request) => {
                worker::prepare(request, service, cancel, status, metadata_budget)
            }
            Self::Rename(request, scope) => worker::prepare_captured_rename(
                request,
                scope,
                service,
                cancel,
                status,
                metadata_budget,
            ),
            Self::Delete(request, scope) => worker::prepare_captured_delete(
                request,
                scope,
                service,
                cancel,
                status,
                metadata_budget,
            ),
        }
    }
}
