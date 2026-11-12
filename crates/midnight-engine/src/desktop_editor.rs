//! Shared bounded editor launch, including backend-only captured source resolution.
use super::*;

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
        let Some(services) = services else {
            self.reject_editor_target(request, target, "External editing is unavailable.")
                .await;
            return;
        };
        let Ok(permit) = self.open_budget.clone().try_acquire_owned() else {
            self.reject_editor_target(request, target, "Editor launch is busy. Try again.")
                .await;
            return;
        };
        let Ok(slot) = self.terminal_slots.clone().try_acquire_owned() else {
            self.reject_editor_target(
                request,
                target,
                "Eight terminal, editor or Custom Command processes are active.",
            )
            .await;
            return;
        };
        let children = self.children.clone();
        let outbox = self.outbox.clone();
        // Transfer provider ownership before the delivery future can be aborted.
        // Admission permits bound even jobs whose blocking worker has not started.
        let worker = tokio::task::spawn_blocking(move || {
            let result = target
                .resolve()
                .and_then(|path| services.launch_editor(&path, &editor));
            drop(target); // Last provider ownership is always released on this worker.
            (result, permit)
        });
        tasks.spawn(async move {
            let result = worker.await;
            let (result, _permit) = match result {
                Ok((Ok(child), permit)) => {
                    let _ = children
                        .send(TrackedChild {
                            child,
                            _slot: Arc::new(slot),
                            completion: None,
                        })
                        .await;
                    (Ok(DesktopActionResult::EditorStarted), Some(permit))
                }
                Ok((Err(error), permit)) => (Err(error), Some(permit)),
                Err(error) => (
                    Err(worker_error("external editor", Path::new(""), error)),
                    None,
                ),
            };
            outbox
                .send(Event::DesktopActionFinished { request, result })
                .await;
        });
    }

    async fn reject_editor_target(
        &self,
        request: DesktopRequestId,
        target: Target,
        message: &'static str,
    ) {
        // Await one cleanup: rejected provider owners cannot accumulate in another queue.
        let _ = tokio::task::spawn_blocking(move || drop(target)).await;
        self.editor_failure(request, message).await;
    }
}

#[cfg(test)]
#[path = "desktop_editor_tests.rs"]
mod tests;
