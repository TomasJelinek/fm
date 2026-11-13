//! Latest-only speculative hover checks. No transfer admission or mutation authority.
use super::*;

pub(super) struct Work {
    request: InternalDropPreviewRequest,
    target: Option<Arc<dyn CapturedDropDirectory>>,
    source_pane: PaneId,
    source_navigation: NavigationId,
    files: Arc<FileClipboard>,
    captured: Option<Arc<dyn CapturedClipboardFiles>>,
    host: Host,
    service: Option<Arc<dyn TransferService>>,
}

pub(super) struct Epoch {
    id: DragId,
    revision: DragPreviewRevision,
    cancel: Cancellation,
}

impl State {
    pub(super) fn clear_preview(&mut self) {
        if let Some(epoch) = self.preview_epoch.take() {
            epoch.cancel.cancel();
        }
        if let Some(work) = self.preview.take() {
            self.retired.push(Box::new(work));
        }
        if let Some(request) = self.pending_preview.take() {
            self.retired.push(Box::new(request));
        }
    }
    fn preview_current(&self, id: DragId, revision: DragPreviewRevision) -> bool {
        self.preview_epoch.as_ref().is_some_and(|epoch| {
            epoch.id == id && epoch.revision == revision && !epoch.cancel.is_cancelled()
        }) && self.source.as_ref().is_some_and(|source| {
            source.id == id && !source.running && !source.released && !source.cancel.is_cancelled()
        })
    }
}

impl Controller {
    pub(crate) async fn preview_internal(
        &self,
        mut request: InternalDropPreviewRequest,
    ) -> Option<TreeCommand> {
        self.retire_backlog().await;
        let command = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let prepared = state.source.as_ref().filter(|source| {
                source.id == request.id
                    && !source.running
                    && !source.released
                    && source.files.is_some()
                    && !source.cancel.is_cancelled()
            });
            if request.revision.0 == 0
                || request.revision <= state.preview_revision
                || request.cancel.is_cancelled()
                || prepared.is_none()
                || state
                    .host
                    .as_ref()
                    .is_none_or(|host| host.host.lifetime.is_cancelled())
            {
                state.retired.push(Box::new(request));
                self.wake_prepare();
                return None;
            }
            request.cancel = prepared
                .unwrap()
                .cancel
                .linked(&request.cancel)
                .linked(&state.host.as_ref().unwrap().host.lifetime);
            state.clear_preview();
            state.preview_revision = request.revision;
            state.preview_epoch = Some(Epoch {
                id: request.id,
                revision: request.revision,
                cancel: request.cancel.clone(),
            });
            if let InternalDropDestination::TreeOccurrence {
                view,
                row,
                request: target_request,
            } = &request.destination
            {
                let command = TreeCommand::CaptureDropDirectory {
                    id: request.id,
                    view: view.clone(),
                    row: *row,
                    request: *target_request,
                };
                state.pending_preview = Some(request);
                Some(command)
            } else {
                self.schedule_preview(&mut state, request, None);
                None
            }
        };
        self.wake_prepare();
        command
    }

    pub(crate) async fn complete_preview(
        &self,
        id: DragId,
        revision: DragPreviewRevision,
        target: Arc<dyn CapturedDropDirectory>,
    ) {
        self.retire_backlog().await;
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let valid = state.preview_current(id, revision) && state.pending_preview.as_ref().is_some_and(|request| {
                request.id == id && request.revision == revision
                    && matches!(&request.destination, InternalDropDestination::TreeOccurrence { view, row, .. } if view.stamp == target.stamp() && *row == target.row())
            });
            if valid {
                let request = state
                    .pending_preview
                    .take()
                    .expect("validated preview capture");
                self.schedule_preview(&mut state, request, Some(target));
            } else {
                state.retired.push(Box::new(target));
            }
        }
        self.wake_prepare();
    }

    fn schedule_preview(
        &self,
        state: &mut State,
        request: InternalDropPreviewRequest,
        target: Option<Arc<dyn CapturedDropDirectory>>,
    ) {
        let source = state.source.as_ref().expect("validated prepared source");
        state.preview = Some(Work {
            request,
            target,
            source_pane: source.pane,
            source_navigation: source.navigation,
            files: source.files.clone().expect("validated source files"),
            captured: source.captured.clone(),
            host: state.host.clone().expect("validated drag host"),
            service: self.transfers.clone(),
        });
    }
}

pub(super) fn execute(
    work: Work,
    state: &SharedState,
    snapshots: &Snapshots,
    events: &mpsc::Sender<DragDropEvent>,
) {
    let cancel = &work.request.cancel;
    let result = (|| {
        let gate = midnight_work_budget::shared_io();
        let _permit = gate
            .acquire(midnight_work_budget::Admission::Exclusive, || {
                cancel.is_cancelled()
            })
            .ok_or_else(|| error(io::ErrorKind::Interrupted, "Hover preview cancelled."))?;
        {
            let snapshots = snapshots.lock().unwrap_or_else(|e| e.into_inner());
            current_source(&snapshots[work.source_pane.index()], work.source_navigation)?;
        }
        if let Some(source) = &work.captured
            && source.resolve(cancel)? != *work.files
        {
            return Err(error(
                io::ErrorKind::InvalidInput,
                "The captured drag sources changed.",
            ));
        }
        let (pane, navigation, destination) = internal::folder(
            snapshots,
            &work.request.destination,
            work.target.as_deref(),
            cancel,
        )?;
        let service = work.service.as_deref().ok_or_else(|| {
            error(
                io::ErrorKind::Unsupported,
                "Native transfers are unavailable.",
            )
        })?;
        validate(service, &work.files, &destination, cancel)?;
        let intent = internal::resolve_intent(
            work.request.intent,
            Some(service),
            &work.files,
            &destination,
            cancel,
        )?;
        {
            let snapshots = snapshots.lock().unwrap_or_else(|e| e.into_inner());
            current_source(&snapshots[work.source_pane.index()], work.source_navigation)?;
            current_source(&snapshots[pane.index()], navigation)?;
        }
        Ok(intent)
    })();
    let current = {
        let state = state.lock().unwrap_or_else(|e| e.into_inner());
        state.preview_current(work.request.id, work.request.revision)
            && state.host.as_ref().is_some_and(|host| {
                host.host.generation == work.host.host.generation
                    && !host.host.lifetime.is_cancelled()
            })
    };
    if current && !cancel.is_cancelled() {
        send_worker_event(
            events,
            DragDropEvent::InternalPreview {
                id: work.request.id,
                revision: work.request.revision,
                intent: result.ok(),
            },
            cancel,
        );
    }
}

pub(super) fn validate(
    service: &dyn TransferService,
    files: &FileClipboard,
    destination: &Path,
    cancel: &Cancellation,
) -> Result<(), FsError> {
    files.validate()?;
    let directory = service.prepare_transfer_destination(destination, cancel)?;
    for path in &files.paths {
        if cancel.is_cancelled() {
            return Err(error(
                io::ErrorKind::Interrupted,
                "Hover preview cancelled.",
            ));
        }
        let source = service.prepare_transfer_source(path, cancel)?;
        source.revalidate(cancel)?;
        let entry = source.entry();
        if !matches!(entry.item.kind, EntryKind::File | EntryKind::Directory) {
            return Err(error(
                io::ErrorKind::Unsupported,
                "Only ordinary files and directories can be dropped.",
            ));
        }
        let identity = source.native_identity().ok_or_else(|| {
            error(
                io::ErrorKind::Unsupported,
                "A native source identity is required.",
            )
        })?;
        if entry.item.kind == EntryKind::Directory {
            directory.reject_source_ancestor(identity, cancel)?;
        }
        let name = path.file_name().ok_or_else(|| {
            error(
                io::ErrorKind::InvalidInput,
                "A source has no native basename.",
            )
        })?;
        if directory
            .inspect(name, cancel)?
            .is_some_and(|target| target.identity == identity)
        {
            return Err(error(
                io::ErrorKind::InvalidInput,
                "The drop would target the source itself.",
            ));
        }
    }
    directory.revalidate(cancel)
}

#[cfg(test)]
#[path = "drag_drop_preview_tests.rs"]
mod tests;
