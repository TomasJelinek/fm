//! Captured tree authority stays on admitted workers through native desktop calls.
use super::*;
use crate::captured_native::{check_cancel, error, resolve};

struct Work {
    source: Arc<dyn CapturedClipboardFiles>,
    cancel: Cancellation,
}

impl Desktop {
    pub(crate) async fn properties_captured(
        &self,
        filesystem: Arc<dyn FileSystem>,
        request: DesktopRequestId,
        source: Arc<dyn CapturedClipboardFiles>,
        cancel: Cancellation,
        tasks: &mut JoinSet<()>,
    ) {
        // EngineRun retains intake ownership while this bounded reservation waits.
        let lease = self.editor_intake.lock().await.reserve().await;
        let work = lease.own(Work { source, cancel });
        let Ok(permit) = self.property_budget.clone().try_acquire_owned() else {
            let retired = work.retire().await;
            self.captured_failure(request, "A properties request is already open.", retired)
                .await;
            return;
        };
        let services = self.services.clone();
        let locale = self.locale.clone();
        let outbox = self.outbox.clone();
        let permit = Arc::new(permit);
        let worker_permit = permit.clone();
        let worker = tokio::task::spawn_blocking(self.scope.blocking(move || {
            let _permit = worker_permit;
            let (payload, lease) = work.into_parts();
            let work = payload;
            let result = (|| {
                let files = resolve(work.source.as_ref(), &work.cancel)?;
                show_properties(
                    filesystem.as_ref(),
                    services.as_deref(),
                    locale.as_deref(),
                    &files.paths,
                    &work.cancel,
                )
            })();
            drop(work);
            drop(lease);
            result
        }));
        tasks.spawn(self.scope.future(async move {
            let _permit = permit;
            let result = match worker.await {
                Ok(result) => result,
                Err(cause) => Err(worker_error("captured properties", Path::new(""), cause)),
            };
            outbox
                .send(Event::DesktopActionFinished { request, result })
                .await;
        }));
    }

    pub(crate) async fn open_captured_entry(
        &self,
        request: DesktopRequestId,
        (pane, navigation): (PaneId, NavigationId),
        source: Arc<dyn CapturedClipboardFiles>,
        cancel: Cancellation,
        row: TreeRowId,
        tasks: &mut JoinSet<()>,
    ) {
        let lease = self.editor_intake.lock().await.reserve().await;
        let work = lease.own(Work { source, cancel });
        let permit = self.open_budget.clone().try_acquire_owned();
        let slot = self.open_slots.clone().try_acquire_owned();
        let ticket = self.processes.openers.try_reserve();
        if self.services.is_none() || permit.is_err() || slot.is_err() || ticket.is_err() {
            let message = if self.services.is_none() {
                "Desktop file opening is unavailable."
            } else {
                "File opening or process cleanup is busy. Try again."
            };
            let retired = work.retire().await;
            self.captured_failure(request, message, retired).await;
            return;
        }
        let services = self.services.clone().expect("admitted desktop service");
        let permit = permit.expect("admitted open permit");
        let slot = Arc::new(slot.expect("admitted process slot"));
        let ticket = ticket.expect("admitted process cleanup");
        let completion = OpenCompletion {
            request,
            pane,
            navigation,
            children: self.children.clone(),
            outbox: self.outbox.clone(),
            slot: slot.clone(),
        };
        let process_scope = Arc::new(self.scope.clone());
        let permit = Arc::new(permit);
        let worker_permit = permit.clone();
        let worker = tokio::task::spawn_blocking(self.scope.blocking(move || {
            let _permit = worker_permit;
            let (payload, lease) = work.into_parts();
            let work = payload;
            let result = (|| {
                let files = resolve(work.source.as_ref(), &work.cancel)?;
                let mut matching = files.paths.iter().enumerate().filter_map(|(index, path)| {
                    work.source
                        .tree_copy_occurrence(index)
                        .filter(|occurrence| {
                            occurrence.row == row
                                && occurrence.stamp.pane == pane
                                && occurrence.stamp.navigation == navigation
                        })
                        .map(|_| path)
                });
                let path = matching
                    .next()
                    .ok_or_else(|| {
                        error(
                            io::ErrorKind::InvalidInput,
                            "The requested row is absent from this capture.",
                        )
                    })?
                    .clone();
                if matching.next().is_some() {
                    return Err(error(
                        io::ErrorKind::InvalidData,
                        "The captured row is ambiguous.",
                    ));
                }
                check_cancel(&work.cancel)?;
                let result = match services.open_entry(&path)? {
                    OpenEntryResult::File {
                        process: Some(child),
                    } => OpenEntryResult::File {
                        process: Some(Box::new(ticket.own(child, slot, process_scope))),
                    },
                    other => other,
                };
                Ok((path, result))
            })();
            drop(work);
            drop(lease);
            result
        }));
        tasks.spawn(self.scope.future(async move {
            let _permit = permit;
            let result = match worker.await {
                Ok(result) => result,
                Err(cause) => Err(worker_error("captured open", Path::new(""), cause)),
            };
            completion.finish(result).await;
        }));
    }

    async fn captured_failure(
        &self,
        request: DesktopRequestId,
        message: &str,
        retired: Result<(), crate::retirement::RetirementPanic>,
    ) {
        let error = retired.err().map_or_else(
            || error(io::ErrorKind::WouldBlock, message),
            |cause| {
                error(
                    io::ErrorKind::Other,
                    &format!("Captured action cleanup failed: {cause}"),
                )
            },
        );
        self.outbox
            .send(Event::DesktopActionFinished {
                request,
                result: Err(error),
            })
            .await;
    }
}

#[cfg(test)]
#[path = "desktop_captured_tests.rs"]
mod tests;
