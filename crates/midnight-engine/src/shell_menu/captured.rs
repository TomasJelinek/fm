//! Exact captured native roots are resolved on the menu worker, never from row labels.
use super::*;
use crate::captured_native::{check_cancel, resolve};

pub(crate) struct CapturedInput {
    pub request: DesktopRequestId,
    pub source: Arc<dyn CapturedClipboardFiles>,
    pub cancel: Cancellation,
    pub owner: usize,
    pub position: ShellMenuPosition,
    pub modifiers: ShellMenuModifiers,
}

impl Controller {
    pub(crate) async fn show_captured(&self, input: CapturedInput, tasks: &mut JoinSet<()>) {
        // EngineRun retains the incoming source until this bounded intake is admitted.
        let lease = self.intake.lock().await.reserve().await;
        let input = lease.own(input);
        let request = input.request;
        let Ok(permit) = self.budget.clone().try_acquire_owned() else {
            let retired = input.retire().await;
            let failure = retired.err().map_or_else(
                || error(io::ErrorKind::WouldBlock, "A system menu is already open."),
                |cause| {
                    error(
                        io::ErrorKind::Other,
                        &format!("Menu capture cleanup failed: {cause}"),
                    )
                },
            );
            self.outbox
                .send(Event::ShellMenuFinished {
                    request,
                    result: Err(failure),
                })
                .await;
            return;
        };
        let service = self.service.clone();
        let cancel = self.cancel.clone();
        let outbox = self.outbox.clone();
        let permit = Arc::new(permit);
        let worker_permit = permit.clone();
        // Start before spawning delivery so aborting that future cannot move source disposal
        // onto the executor or free a still-running native worker's admission.
        let worker = tokio::task::spawn_blocking(self.scope.blocking(move || {
            let _permit = worker_permit;
            let (payload, lease) = input.into_parts();
            let input = payload;
            let result = (|| {
                check_cancel(&cancel)?;
                let files = resolve(input.source.as_ref(), &input.cancel)?;
                check_cancel(&cancel)?;
                let parent = files.paths[0].parent();
                if files.paths.iter().any(|path| path.parent() != parent) {
                    return Ok(ShellMenuOutcome::Unavailable);
                }
                let request = ShellMenuRequest {
                    paths: files.paths,
                    owner: input.owner,
                    position: input.position,
                    modifiers: input.modifiers,
                    target: ShellMenuTarget::Items,
                };
                request.validate()?;
                check_cancel(&cancel)?;
                check_cancel(&input.cancel)?;
                service.map_or(Ok(ShellMenuOutcome::Unavailable), |service| {
                    service.show_shell_menu(&request)
                })
            })();
            drop(input);
            drop(lease);
            result
        }));
        tasks.spawn(self.scope.future(async move {
            let result = match worker.await {
                Ok(result) => result,
                Err(cause) => Err(error(io::ErrorKind::Other, &cause.to_string())),
            };
            outbox
                .send(Event::ShellMenuFinished { request, result })
                .await;
            drop(permit);
        }));
    }
}

#[cfg(test)]
mod tests;
