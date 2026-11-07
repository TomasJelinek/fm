//! Shared desktop process admission and tracking, without history/ordinary-terminal actions.
use crate::desktop::TrackedChild;
use midnight_core::{Cancellation, FsError, TerminalProcess};
use std::{io, path::Path, sync::Arc, time::Duration};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};

#[derive(Clone)]
pub(crate) struct Runtime {
    launches: Arc<Semaphore>,
    processes: Arc<Semaphore>,
    children: mpsc::Sender<TrackedChild>,
}
pub(crate) struct Reservation {
    pub(crate) launch: OwnedSemaphorePermit,
    pub(crate) process: OwnedSemaphorePermit,
}
impl Runtime {
    pub(crate) fn new(
        launches: Arc<Semaphore>,
        processes: Arc<Semaphore>,
        children: mpsc::Sender<TrackedChild>,
    ) -> Self {
        Self {
            launches,
            processes,
            children,
        }
    }
    pub(crate) async fn reserve(&self, cancel: &Cancellation) -> Result<Reservation, FsError> {
        if self.children.is_closed() {
            return Err(failure("Process tracking is unavailable."));
        }
        let process = acquire(self.processes.clone(), cancel).await?;
        let launch = acquire(self.launches.clone(), cancel).await?;
        if cancel.is_cancelled() || self.children.is_closed() {
            return Err(failure("Command launch cancelled or tracking unavailable."));
        }
        Ok(Reservation { launch, process })
    }
    pub(crate) async fn track(
        &self,
        child: Box<dyn TerminalProcess>,
        slot: OwnedSemaphorePermit,
    ) -> Result<(), FsError> {
        if let Err(error) = self
            .children
            .send(TrackedChild {
                child,
                _slot: slot,
                completion: None,
            })
            .await
        {
            let TrackedChild {
                mut child,
                _slot: slot,
                ..
            } = error.0;
            // A started child is never killed or discarded because the UI/reaper closed.
            // At most eight owners retry at a timed interval, just like the normal reaper.
            // A wait error does not establish exit and must not release process admission.
            tokio::spawn(async move {
                let _slot = slot;
                while !matches!(child.try_wait(), Ok(Some(_))) {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            });
            return Err(failure(
                "Program started, but its normal process tracker closed.",
            ));
        }
        Ok(())
    }
}
async fn acquire(
    semaphore: Arc<Semaphore>,
    cancel: &Cancellation,
) -> Result<OwnedSemaphorePermit, FsError> {
    loop {
        if cancel.is_cancelled() {
            return Err(FsError::from_io(
                "Custom Command",
                Path::new(""),
                io::ErrorKind::Interrupted.into(),
            ));
        }
        match tokio::time::timeout(Duration::from_millis(25), semaphore.clone().acquire_owned())
            .await
        {
            Ok(Ok(permit)) => return Ok(permit),
            Ok(Err(_)) => return Err(failure("Process admission is closing.")),
            Err(_) => {}
        }
    }
}
fn failure(message: &str) -> FsError {
    FsError::from_io(
        "Custom Command",
        Path::new(""),
        io::Error::new(io::ErrorKind::BrokenPipe, message),
    )
}

#[cfg(test)]
#[path = "user_command_runtime_tests.rs"]
mod tests;
