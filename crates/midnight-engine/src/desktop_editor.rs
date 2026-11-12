//! Shared bounded editor launch, including backend-only captured source resolution.
use super::*;
use crate::{retirement::WorkerOwned, user_command_runtime::ProcessTicket};

struct Work {
    target: Target,
    launch: Option<Launch>,
}
struct Launch {
    services: Arc<dyn EditorService>,
    scope: TaskLifetime,
    permit: Option<OwnedSemaphorePermit>,
    ticket: Option<ProcessTicket>,
    slot: Arc<OwnedSemaphorePermit>,
}

enum Target {
    Native(PathBuf),
    Captured {
        source: Arc<dyn CapturedEditorFile>,
        cancel: Cancellation,
    },
}
impl Target {
    fn resolve(&self) -> Result<PathBuf, FsError> {
        match self {
            Self::Native(path) => Ok(path.clone()),
            Self::Captured { source, cancel } => {
                let check = || {
                    if cancel.is_cancelled() {
                        Err(FsError::from_io(
                            "external editor",
                            Path::new(""),
                            io::ErrorKind::Interrupted.into(),
                        ))
                    } else {
                        Ok(())
                    }
                };
                check()?;
                let path = source.resolve(cancel)?;
                check()?;
                Ok(path)
            }
        }
    }
}

impl Desktop {
    pub(crate) async fn edit_file(
        &self,
        services: Option<Arc<dyn EditorService>>,
        request: DesktopRequestId,
        path: PathBuf,
        editor: EditorPreferences,
        tasks: &mut JoinSet<()>,
    ) {
        self.edit_target(services, request, Target::Native(path), editor, tasks)
            .await;
    }

    pub(crate) async fn edit_captured_file(
        &self,
        services: Option<Arc<dyn EditorService>>,
        request: DesktopRequestId,
        source: Arc<dyn CapturedEditorFile>,
        editor: EditorPreferences,
        cancel: Cancellation,
        tasks: &mut JoinSet<()>,
    ) {
        self.edit_target(
            services,
            request,
            Target::Captured { source, cancel },
            editor,
            tasks,
        )
        .await;
    }

    async fn edit_target(
        &self,
        services: Option<Arc<dyn EditorService>>,
        request: DesktopRequestId,
        target: Target,
        editor: EditorPreferences,
        tasks: &mut JoinSet<()>,
    ) {
        // EngineRun protects the incoming capture while this single intake waits.
        // The lease follows the same target through rejection or native execution.
        let lease = self.editor_intake.lock().await.reserve().await;
        let mut work = lease.own(Work {
            target,
            launch: None,
        });
        let Some(services) = services else {
            self.reject_editor_target(request, work, "External editing is unavailable.")
                .await;
            return;
        };
        let Ok(permit) = self.open_budget.clone().try_acquire_owned() else {
            self.reject_editor_target(request, work, "Editor launch is busy. Try again.")
                .await;
            return;
        };
        let Ok(slot) = self.terminal_slots.clone().try_acquire_owned() else {
            self.reject_editor_target(
                request,
                work,
                "Eight terminal, editor or Custom Command processes are active.",
            )
            .await;
            return;
        };
        let Ok(ticket) = self.processes.terminals.try_reserve() else {
            self.reject_editor_target(request, work, "Editor process cleanup is busy. Try again.")
                .await;
            return;
        };
        work.launch = Some(Launch {
            services,
            scope: self.scope.clone(),
            permit: Some(permit),
            ticket: Some(ticket),
            slot: Arc::new(slot),
        });
        let children = self.children.clone();
        let outbox = self.outbox.clone();
        let worker = tokio::task::spawn_blocking(self.scope.blocking(move || {
            let (payload, lease) = work.into_parts();
            // Binding order keeps the lease until all raw payload destruction,
            // including provider unwinding, has finished on this native worker.
            let mut work = payload;
            let result = work.target.resolve().and_then(|path| {
                let launch = work.launch.as_mut().expect("admitted editor owns launch");
                launch.services.launch_editor(&path, &editor).map(|child| {
                    launch
                        .ticket
                        .take()
                        .expect("admitted editor owns ticket")
                        .own(child, launch.slot.clone(), Arc::new(launch.scope.clone()))
                })
            });
            let permit = work.launch.as_mut().and_then(|launch| launch.permit.take());
            drop(work);
            drop(lease);
            (result, permit)
        }));
        tasks.spawn(self.scope.future(async move {
            let result = worker.await;
            let (result, _permit) = match result {
                Ok((Ok(child), permit)) => {
                    let _ = children
                        .send(TrackedChild {
                            _slot: child.quota(),
                            child: Box::new(child),
                            completion: None,
                        })
                        .await;
                    (Ok(DesktopActionResult::EditorStarted), permit)
                }
                Ok((Err(error), permit)) => (Err(error), permit),
                Err(error) => (
                    Err(worker_error("external editor", Path::new(""), error)),
                    None,
                ),
            };
            outbox
                .send(Event::DesktopActionFinished { request, result })
                .await;
        }));
    }

    async fn reject_editor_target(
        &self,
        request: DesktopRequestId,
        target: WorkerOwned<Work>,
        message: &'static str,
    ) {
        // The fixed admission stays occupied until actual provider destruction;
        // cancellation needs no Tokio runtime or replacement cleanup task.
        if let Err(error) = target.retire().await {
            self.editor_failure(request, &format!("Editor cleanup failed: {error}"))
                .await;
            return;
        }
        self.editor_failure(request, message).await;
    }
}

#[cfg(test)]
#[path = "desktop_editor_tests.rs"]
mod tests;
