//! Explicit Find actions validate their retained authority in the native launch worker.
use super::*;

struct Work {
    source: Arc<dyn CapturedFindFile>,
    cancel: Cancellation,
    action: FindFileOperation,
}

impl Desktop {
    pub(crate) async fn execute_find_file(
        &self,
        editor_service: Option<Arc<dyn EditorService>>,
        request: DesktopRequestId,
        source: Arc<dyn CapturedFindFile>,
        action: FindFileOperation,
        cancel: Cancellation,
        tasks: &mut JoinSet<()>,
    ) {
        let lease = self.editor_intake.lock().await.reserve().await;
        let work = lease.own(Work {
            source,
            cancel,
            action,
        });
        let editing = matches!(work.action, FindFileOperation::Edit(_));
        let unavailable = if editing {
            editor_service.is_none()
        } else {
            self.services.is_none()
        };
        let permit = self.open_budget.clone().try_acquire_owned();
        let slot = if editing {
            self.terminal_slots.clone()
        } else {
            self.open_slots.clone()
        }
        .try_acquire_owned();
        let ticket = if editing {
            &self.processes.terminals
        } else {
            &self.processes.openers
        }
        .try_reserve();
        if unavailable || permit.is_err() || slot.is_err() || ticket.is_err() {
            let message = if unavailable {
                "The requested native file action is unavailable."
            } else {
                "File action or process cleanup capacity is busy. Try again."
            };
            let result = work.retire().await;
            self.editor_failure(
                request,
                &result.err().map_or_else(
                    || message.to_owned(),
                    |error| format!("Find action cleanup failed: {error}"),
                ),
            )
            .await;
            return;
        }
        let permit = permit.expect("admitted launch permit");
        let slot = Arc::new(slot.expect("admitted process slot"));
        let ticket = ticket.expect("admitted process cleanup ticket");
        let services = self.services.clone();
        let scope = self.scope.clone();
        let process_scope = Arc::new(scope.clone());
        let path = work.source.entry().path.clone();
        let tracking_path = path.clone();
        let tracking_slot = slot.clone();
        let children = self.children.clone();
        let outbox = self.outbox.clone();
        let worker = tokio::task::spawn_blocking(scope.blocking(move || {
            let _permit = permit;
            let (payload, lease) = work.into_parts();
            let work = payload;
            let result = (|| {
                if work.source.entry().kind != EntryKind::File {
                    return Err(find_error(
                        &path,
                        io::ErrorKind::InvalidInput,
                        "Only regular Find files can be opened or edited.",
                    ));
                }
                work.source.validate(&work.cancel)?;
                if work.cancel.is_cancelled() {
                    return Err(find_error(
                        &path,
                        io::ErrorKind::Interrupted,
                        "Find action cancelled.",
                    ));
                }
                let process = match &work.action {
                    FindFileOperation::Edit(editor) => Some(
                        editor_service
                            .as_ref()
                            .expect("admitted editor service")
                            .launch_editor(&path, editor)?,
                    ),
                    FindFileOperation::Open => match services
                        .as_ref()
                        .expect("admitted desktop service")
                        .open_entry(&path)?
                    {
                        OpenEntryResult::File { process } => process,
                        _ => {
                            return Err(find_error(
                                &path,
                                io::ErrorKind::InvalidData,
                                "Find file changed to a folder before native activation.",
                            ));
                        }
                    },
                };
                Ok(process.map(|child| {
                    Box::new(ticket.own(child, slot, process_scope)) as Box<dyn TerminalProcess>
                }))
            })();
            drop(work);
            drop(lease);
            result
        }));
        tasks.spawn(self.scope.future(async move {
            let result = match worker.await {
                Ok(Ok(process)) => {
                    if let Some(child) = process {
                        let _ = children
                            .send(TrackedChild {
                                child,
                                _slot: tracking_slot,
                                completion: (!editing).then_some((request, tracking_path)),
                            })
                            .await;
                        if !editing {
                            return;
                        }
                    }
                    Ok(if editing {
                        DesktopActionResult::EditorStarted
                    } else {
                        DesktopActionResult::FileOpened
                    })
                }
                Ok(Err(error)) => Err(error),
                Err(error) => Err(worker_error("Find file action", &tracking_path, error)),
            };
            outbox
                .send(Event::DesktopActionFinished { request, result })
                .await;
        }));
    }
}

#[cfg(test)]
#[path = "desktop_find_tests.rs"]
mod tests;
