//! Typed internal destinations resolve on the coalescing backend worker.
use super::*;
use std::path::PathBuf;

pub(super) struct Work {
    request: InternalDropRequest,
    target: Option<Arc<dyn CapturedDropDirectory>>,
    source_pane: PaneId,
    source_navigation: NavigationId,
    files: Arc<FileClipboard>,
    captured: Option<Arc<dyn CapturedClipboardFiles>>,
    cancel: Cancellation,
    host: Host,
    service: Option<Arc<dyn TransferService>>,
}

impl Controller {
    pub(crate) async fn internal_request(
        &self,
        request: InternalDropRequest,
    ) -> Option<TreeCommand> {
        self.retire_backlog().await;
        let id = request.id;
        let command = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let valid = state.pending_internal.is_none()
                && state.internal.is_none()
                && state.source.as_ref().is_some_and(|source| {
                    source.id == request.id
                        && !source.running
                        && !source.released
                        && source.files.is_some()
                        && !source.cancel.is_cancelled()
                });
            if !valid {
                None
            } else if let InternalDropDestination::TreeOccurrence {
                view,
                row,
                request: target_request,
            } = &request.destination
            {
                state.source.as_mut().expect("validated source").released = true;
                let command = TreeCommand::CaptureDropDirectory {
                    id: request.id,
                    view: view.clone(),
                    row: *row,
                    request: *target_request,
                };
                state.pending_internal = Some(request);
                return Some(command);
            } else {
                state.source.as_mut().expect("validated source").released = true;
                Self::schedule_internal(&mut state, request, None, self.transfers.clone())
            }
        };
        if command.is_some() {
            self.wake_prepare();
        } else {
            self.failed(
                Some(id),
                error(
                    io::ErrorKind::InvalidInput,
                    "The internal drag is no longer current.",
                ),
            )
            .await;
        }
        None
    }

    pub(crate) async fn complete_internal(
        &self,
        id: DragId,
        target: Arc<dyn CapturedDropDirectory>,
    ) {
        self.retire_backlog().await;
        let scheduled = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let valid = state.pending_internal.as_ref().is_some_and(|request| {
                request.id == id
                    && matches!(&request.destination,
                    InternalDropDestination::TreeOccurrence { view, row, .. }
                    if view.stamp == target.stamp() && *row == target.row())
            });
            if valid {
                let request = state
                    .pending_internal
                    .take()
                    .expect("validated pending drop");
                Self::schedule_internal(&mut state, request, Some(target), self.transfers.clone())
                    .is_some()
            } else {
                if state
                    .pending_internal
                    .as_ref()
                    .is_some_and(|request| request.id == id)
                {
                    state.clear_source();
                }
                state.retired.push(Box::new(target));
                false
            }
        };
        self.wake_prepare();
        if !scheduled {
            self.failed(
                Some(id),
                error(
                    io::ErrorKind::InvalidInput,
                    "The captured drop folder is no longer current.",
                ),
            )
            .await;
        }
    }

    fn schedule_internal(
        state: &mut State,
        request: InternalDropRequest,
        target: Option<Arc<dyn CapturedDropDirectory>>,
        service: Option<Arc<dyn TransferService>>,
    ) -> Option<()> {
        let prepared = state
            .source
            .as_ref()
            .filter(|source| {
                source.id == request.id && !source.running && !source.cancel.is_cancelled()
            })
            .and_then(|source| {
                Some((
                    source.pane,
                    source.navigation,
                    source.files.clone()?,
                    source.captured.clone(),
                    source.cancel.clone(),
                    state.host.clone()?,
                ))
            });
        let Some((source_pane, source_navigation, files, captured, cancel, host)) = prepared else {
            state.retired.push(Box::new(target));
            return None;
        };
        if let Some(old) = state.internal.take() {
            state.retired.push(Box::new(old));
        }
        state.internal = Some(Work {
            service,
            request,
            target,
            source_pane,
            source_navigation,
            files,
            captured,
            cancel,
            host,
        });
        Some(())
    }
}

fn folder(
    snapshots: &Snapshots,
    target: &InternalDropDestination,
    captured: Option<&dyn CapturedDropDirectory>,
    cancel: &Cancellation,
) -> Result<(PaneId, NavigationId, PathBuf), FsError> {
    match target {
        InternalDropDestination::PaneRoot { pane, navigation } => {
            let snapshots = snapshots.lock().unwrap_or_else(|e| e.into_inner());
            Ok((
                *pane,
                *navigation,
                current_source(&snapshots[pane.index()], *navigation)?
                    .path
                    .clone(),
            ))
        }
        InternalDropDestination::PanelEntry {
            pane,
            navigation,
            entry,
        } => {
            let snapshots = snapshots.lock().unwrap_or_else(|e| e.into_inner());
            let snapshot = current_source(&snapshots[pane.index()], *navigation)?;
            let entry = snapshot
                .entries
                .iter()
                .find(|candidate| candidate.id == *entry && candidate.kind == EntryKind::Directory)
                .ok_or_else(|| {
                    error(
                        io::ErrorKind::InvalidInput,
                        "The hovered native folder changed.",
                    )
                })?;
            if entry.path.parent() != Some(snapshot.path.as_path())
                || entry.path.file_name().is_none()
            {
                return Err(error(
                    io::ErrorKind::InvalidInput,
                    "The drop folder is not a child of the acknowledged panel.",
                ));
            }
            Ok((*pane, *navigation, entry.path.clone()))
        }
        InternalDropDestination::TreeOccurrence { view, row, .. } => {
            let target = captured
                .filter(|target| target.stamp() == view.stamp && target.row() == *row)
                .ok_or_else(|| {
                    error(
                        io::ErrorKind::InvalidInput,
                        "The exact tree folder was not captured.",
                    )
                })?;
            let path = target.resolve(cancel)?;
            let snapshots = snapshots.lock().unwrap_or_else(|e| e.into_inner());
            current_source(&snapshots[view.stamp.pane.index()], view.stamp.navigation)?;
            Ok((view.stamp.pane, view.stamp.navigation, path))
        }
    }
}

pub(super) fn execute(
    work: Work,
    state: &SharedState,
    snapshots: &Snapshots,
    events: &mpsc::Sender<DragDropEvent>,
    shutdown: &Cancellation,
) {
    let result = (|| {
        if work.cancel.is_cancelled() || work.host.host.lifetime.is_cancelled() {
            return Err(error(
                io::ErrorKind::Interrupted,
                "Internal drag cancelled.",
            ));
        }
        {
            let snapshots = snapshots.lock().unwrap_or_else(|e| e.into_inner());
            current_source(&snapshots[work.source_pane.index()], work.source_navigation)?;
        }
        if let Some(source) = &work.captured
            && source.resolve(&work.cancel)? != *work.files
        {
            return Err(error(
                io::ErrorKind::InvalidInput,
                "The captured drag sources changed.",
            ));
        }
        let (pane, navigation, destination) = folder(
            snapshots,
            &work.request.destination,
            work.target.as_deref(),
            &work.cancel,
        )?;
        let intent = resolve_intent(
            work.request.intent,
            work.service.as_deref(),
            &work.files,
            &destination,
            &work.cancel,
        )?;
        let files = (*work.files).clone();
        files.validate()?;
        let retention = (work.captured.is_some() || work.target.is_some()).then(|| {
            Arc::new(Retention {
                _source: work.captured.clone(),
                _target: work.target.clone(),
                completions: Arc::new(
                    work.files
                        .paths
                        .iter()
                        .enumerate()
                        .filter_map(|(index, path)| {
                            Some((
                                path.clone(),
                                work.captured.as_ref()?.tree_copy_occurrence(index)?,
                            ))
                        })
                        .collect(),
                ),
            })
        });
        let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
        if work.cancel.is_cancelled()
            || state
                .source
                .as_ref()
                .is_none_or(|source| source.id != work.request.id || source.running)
            || state.host.as_ref().is_none_or(|host| {
                host.host.generation != work.host.host.generation
                    || host.host.lifetime.is_cancelled()
            })
        {
            return Err(error(
                io::ErrorKind::Interrupted,
                "The internal drag was superseded.",
            ));
        }
        let snapshots = snapshots.lock().unwrap_or_else(|e| e.into_inner());
        current_source(&snapshots[work.source_pane.index()], work.source_navigation)?;
        current_source(&snapshots[pane.index()], navigation)?;
        let permit = work.host.sink.capacity.reserve()?;
        let token = DropToken(
            work.host
                .sink
                .next
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                    value.checked_add(1)
                })
                .map_err(|_| error(io::ErrorKind::Other, "Drop token sequence exhausted."))?
                + 1,
        );
        let destination = Arc::new(destination);
        let event = DragDropEvent::DropAccepted {
            token,
            drag: Some(work.request.id),
            pane,
            item_count: files.paths.len(),
            destination: destination.clone(),
            intent,
            verification: Some(work.request.verification),
        };
        let mut pending = work
            .host
            .sink
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        pending.insert(
            token,
            Accepted {
                files,
                destination,
                intent,
                verification: Some(work.request.verification),
                permit,
                retention,
            },
        );
        if events.try_send(event).is_err() {
            pending.remove(&token);
            return Err(error(
                io::ErrorKind::WouldBlock,
                "The application cannot receive this drop.",
            ));
        }
        state.clear_source();
        Ok(intent)
    })();
    let event = match result {
        Ok(intent) => DragDropEvent::Finished {
            id: work.request.id,
            outcome: if intent == TransferIntent::Move {
                DragFinish::Moved
            } else {
                DragFinish::Copied
            },
        },
        Err(error) => {
            let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
            let current = state
                .source
                .as_ref()
                .is_some_and(|source| source.id == work.request.id);
            let retired =
                !current || work.cancel.is_cancelled() || work.host.host.lifetime.is_cancelled();
            if current {
                state.clear_source();
            }
            // Cancellation already published its terminal reply. A blocked provider
            // returning later must not replace that outcome or a newer gesture's status.
            if retired {
                return;
            }
            DragDropEvent::Failed {
                id: Some(work.request.id),
                error,
            }
        }
    };
    send_worker_event(events, event, shutdown);
}

pub(super) fn resolve_intent(
    intent: InternalDropIntent,
    service: Option<&dyn TransferService>,
    files: &FileClipboard,
    destination: &Path,
    cancel: &Cancellation,
) -> Result<TransferIntent, FsError> {
    if cancel.is_cancelled() {
        return Err(error(
            io::ErrorKind::Interrupted,
            "Internal drag cancelled.",
        ));
    }
    match intent {
        InternalDropIntent::Copy => return Ok(TransferIntent::Copy),
        InternalDropIntent::Move => return Ok(TransferIntent::Move),
        InternalDropIntent::Auto => {}
    }
    let device = |path: &Path| -> Result<Option<u64>, FsError> {
        let result = service.map(|service| service.native_drop_device(path, cancel));
        if cancel.is_cancelled() {
            return Err(error(
                io::ErrorKind::Interrupted,
                "Internal drag cancelled.",
            ));
        }
        match result {
            Some(Err(error)) if error.kind == io::ErrorKind::Interrupted => Err(error),
            Some(Ok(device)) => Ok(device),
            _ => Ok(None),
        }
    };
    let Some(target) = device(destination)? else {
        return Ok(TransferIntent::Copy);
    };
    if files.paths.is_empty() {
        return Ok(TransferIntent::Copy);
    }
    for source in &files.paths {
        if device(source)? != Some(target) {
            return Ok(TransferIntent::Copy);
        }
    }
    Ok(TransferIntent::Move)
}
