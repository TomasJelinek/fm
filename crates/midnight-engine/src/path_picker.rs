//! One native chooser at a time, independent of navigation and filesystem workers.
use crate::{Outbox, task_lifetime::TaskLifetime};
use midnight_core::{
    Cancellation, DesktopRequestId, Event, FsError, PathPickerRequest, PathPickerService,
    validate_picked_path,
};
use std::{io, path::Path, sync::Arc};
use tokio::{sync::Semaphore, task::JoinSet};

pub(crate) struct Controller {
    service: Option<Arc<dyn PathPickerService>>,
    budget: Arc<Semaphore>,
    shutdown: Cancellation,
    active: Option<(DesktopRequestId, Cancellation)>,
    outbox: Outbox,
    scope: TaskLifetime,
}

impl Controller {
    pub(crate) fn new(
        scope: TaskLifetime,
        service: Option<Arc<dyn PathPickerService>>,
        outbox: Outbox,
    ) -> Self {
        Self {
            service,
            outbox,
            budget: Arc::new(Semaphore::new(1)),
            shutdown: Cancellation::default(),
            active: None,
            scope,
        }
    }

    pub(crate) async fn show(
        &mut self,
        request: DesktopRequestId,
        input: PathPickerRequest,
        tasks: &mut JoinSet<()>,
    ) {
        let admitted = input.validate().and_then(|()| {
            self.budget
                .clone()
                .try_acquire_owned()
                .map_err(|_| error(io::ErrorKind::WouldBlock, "A file chooser is already open."))
        });
        let permit = match admitted {
            Ok(permit) => permit,
            Err(cause) => {
                self.outbox
                    .send(Event::PathPicked {
                        request,
                        result: Err(cause),
                    })
                    .await;
                return;
            }
        };
        let cancel = self.shutdown.child();
        self.active = Some((request, cancel.clone()));
        let shutdown = self.shutdown.clone();
        let service = self.service.clone();
        let outbox = self.outbox.clone();
        tasks.spawn_blocking(self.scope.blocking(move || {
            // Admission stays owned until the actual native chooser and reply have finished.
            let _permit = permit;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if cancel.is_cancelled() {
                    return Ok(None);
                }
                let service = service.ok_or_else(|| {
                    error(
                        io::ErrorKind::Unsupported,
                        "This platform has no file chooser. Enter the path manually.",
                    )
                })?;
                let result = service.pick_path(&input, &cancel)?;
                if let Some(path) = &result {
                    validate_picked_path(path)?;
                }
                Ok(result)
            }))
            .unwrap_or_else(|_| {
                Err(error(
                    io::ErrorKind::Other,
                    "The native file chooser stopped unexpectedly.",
                ))
            });
            outbox.blocking_send(Event::PathPicked { request, result }, &shutdown);
        }));
    }

    pub(crate) fn cancel(&self, request: DesktopRequestId) {
        if let Some((active, cancel)) = &self.active
            && *active == request
        {
            cancel.cancel();
        }
    }

    pub(crate) fn shutdown(&self) {
        self.shutdown.cancel();
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn error(kind: io::ErrorKind, message: &str) -> FsError {
    FsError::from_io("choose path", Path::new(""), io::Error::new(kind, message))
}

#[cfg(test)]
#[path = "path_picker_tests.rs"]
mod tests;
