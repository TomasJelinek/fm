//! Menus use a separate bounded worker; shell extension code never runs on the UI thread.
use crate::{Outbox, file_targets, task_lifetime::TaskLifetime};
use midnight_core::*;
use std::{io, path::Path, sync::Arc};
use tokio::{sync::Semaphore, task::JoinSet};

mod captured;
pub(crate) use captured::CapturedInput;

// One admitted native worker and one serial dispatch/rejection owner.
pub(crate) const RETIREMENT_CAPACITY: usize = 2;
pub(crate) struct Retirement {
    pub(crate) intake: crate::retirement::RetirementWaiter,
}
#[cfg(test)]
impl Retirement {
    pub(crate) fn testing() -> Self {
        let domain =
            crate::retirement::RetirementDomain::try_start(RETIREMENT_CAPACITY, 1).unwrap();
        Self {
            intake: domain.new_waiter().unwrap(),
        }
    }
}

pub(crate) struct Input {
    pub request: DesktopRequestId,
    pub files: SelectedFiles,
    pub owner: usize,
    pub position: ShellMenuPosition,
    pub modifiers: ShellMenuModifiers,
    pub target: ShellMenuTarget,
}
pub(crate) struct Controller {
    service: Option<Arc<dyn ShellMenuService>>,
    budget: Arc<Semaphore>,
    cancel: Cancellation,
    outbox: Outbox,
    scope: TaskLifetime,
    intake: tokio::sync::Mutex<crate::retirement::RetirementWaiter>,
}
impl Controller {
    pub(crate) fn new(
        scope: TaskLifetime,
        retirement: Retirement,
        service: Option<Arc<dyn ShellMenuService>>,
        outbox: Outbox,
    ) -> Self {
        Self {
            service,
            outbox,
            budget: Arc::new(Semaphore::new(1)),
            cancel: Cancellation::default(),
            scope,
            intake: tokio::sync::Mutex::new(retirement.intake),
        }
    }
    #[cfg(test)]
    fn testing(service: Option<Arc<dyn ShellMenuService>>, outbox: Outbox) -> Self {
        Self::new(
            TaskLifetime::testing(service.clone()),
            Retirement::testing(),
            service,
            outbox,
        )
    }
    pub(crate) async fn show(&self, input: Input, tasks: &mut JoinSet<()>) {
        let Ok(permit) = self.budget.clone().try_acquire_owned() else {
            // Apply result-channel backpressure directly instead of accumulating reply tasks.
            self.outbox
                .send(Event::ShellMenuFinished {
                    request: input.request,
                    result: Err(error(
                        io::ErrorKind::WouldBlock,
                        "A system menu is already open.",
                    )),
                })
                .await;
            return;
        };
        let outbox = self.outbox.clone();
        let service = self.service.clone();
        let cancel = self.cancel.clone();
        let permit = Arc::new(permit);
        let scope = self.scope.clone();
        tasks.spawn(self.scope.future(async move {
            let request = input.request;
            let worker_permit = permit.clone();
            let worker = tokio::task::spawn_blocking(scope.blocking(move || {
                // Keep native work admitted even if its asynchronous reply task is aborted.
                let _permit = worker_permit;
                (|| {
                    let paths = file_targets::resolve(
                        &input.files.selection,
                        &input.files.view,
                        input.files.fallback.as_deref(),
                        &cancel,
                    )?;
                    let request = ShellMenuRequest {
                        paths,
                        owner: input.owner,
                        position: input.position,
                        modifiers: input.modifiers,
                        target: input.target,
                    };
                    request.validate()?;
                    service.map_or(Ok(ShellMenuOutcome::Unavailable), |service| {
                        service.show_shell_menu(&request)
                    })
                })()
            }))
            .await;
            let result = match worker {
                Ok(result) => result,
                Err(failure) => Err(error(io::ErrorKind::Other, &failure.to_string())),
            };
            // The reply retains admission through delivery even when native code panics.
            outbox
                .send(Event::ShellMenuFinished { request, result })
                .await;
            drop(permit);
        }));
    }
    pub(crate) fn shutdown(&self) {
        self.cancel.cancel();
    }
}
fn error(kind: io::ErrorKind, text: &str) -> FsError {
    FsError::from_io("system menu", Path::new(""), io::Error::new(kind, text))
}

#[cfg(test)]
mod tests;
