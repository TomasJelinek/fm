//! Shared desktop process admission and tracking, without history/ordinary-terminal actions.
use crate::desktop::TrackedChild;
use midnight_core::{Cancellation, FsError};
use std::{io, path::Path, sync::Arc, time::Duration};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};

#[path = "user_command_process_tracker.rs"]
mod tracker;
pub(crate) use tracker::{MAX_OPEN_PROCESSES, MAX_PROCESSES, ProcessTracker};
pub(crate) use tracker::{ProcessTicket, TrackedProcess};

#[derive(Clone)]
pub(crate) struct Runtime {
    launches: Arc<Semaphore>,
    processes: Arc<Semaphore>,
    children: mpsc::Sender<TrackedChild>,
    tracker: ProcessTracker,
}
pub(crate) struct Reservation {
    pub(crate) ticket: ProcessTicket,
    pub(crate) launch: OwnedSemaphorePermit,
    pub(crate) process: Arc<OwnedSemaphorePermit>,
}
impl Runtime {
    pub(crate) fn new(
        launches: Arc<Semaphore>,
        processes: Arc<Semaphore>,
        children: mpsc::Sender<TrackedChild>,
        tracker: ProcessTracker,
    ) -> Self {
        Self {
            launches,
            processes,
            children,
            tracker,
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
        let ticket = self.tracker.try_reserve().map_err(|error| {
            FsError::from_io("Custom Command process cleanup", Path::new(""), error)
        })?;
        Ok(Reservation {
            ticket,
            launch,
            process: Arc::new(process),
        })
    }
    pub(crate) async fn track(&self, child: TrackedProcess) -> Result<(), FsError> {
        let slot = child.quota();
        if self
            .children
            .send(TrackedChild {
                child: Box::new(child),
                _slot: slot,
                completion: None,
            })
            .await
            .is_err()
        {
            // The returned tracked owner submits to the already-reserved plain
            // tracker. Runtime loss cannot discard the child or its process quota.
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
