//! Bounded report snapshots; native file ownership never leaves backend workers.
use crate::Outbox;
use midnight_core::{Cancellation, DesktopRequestId, Event, FsError, SizeWarningReport};
use std::{
    io,
    path::Path,
    sync::{Arc, Mutex},
};
use tokio::{sync::mpsc, task::JoinSet};

const WORKERS: usize = 2;
const PENDING: usize = 8;

struct Request {
    id: DesktopRequestId,
    report: SizeWarningReport,
    cancel: Cancellation,
}

pub(crate) struct Controller {
    sender: Option<mpsc::Sender<Request>>,
    stop: Cancellation,
    outbox: Outbox,
}

impl Controller {
    pub(crate) fn new(outbox: Outbox) -> Self {
        Self {
            sender: None,
            stop: Cancellation::default(),
            outbox,
        }
    }

    fn start(&mut self, tasks: &mut JoinSet<()>) -> mpsc::Sender<Request> {
        let (sender, receiver) = mpsc::channel::<Request>(PENDING);
        let receiver = Arc::new(Mutex::new(receiver));
        for _ in 0..WORKERS {
            let receiver = receiver.clone();
            let stop = self.stop.clone();
            let outbox = self.outbox.clone();
            tasks.spawn_blocking(move || {
                loop {
                    // The mutex serializes dequeue only, not snapshot I/O.
                    let request = receiver
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .blocking_recv();
                    let Some(request) = request else { break };
                    if stop.is_cancelled() {
                        continue;
                    }
                    let cancel = request.cancel.linked(&stop);
                    let result = if cancel.is_cancelled() {
                        Err(error(
                            io::ErrorKind::Interrupted,
                            "Report request cancelled",
                        ))
                    } else {
                        request.report.snapshot(&cancel)
                    };
                    let result = if cancel.is_cancelled() {
                        Err(error(
                            io::ErrorKind::Interrupted,
                            "Report request cancelled",
                        ))
                    } else {
                        result
                    };
                    outbox.blocking_send(
                        Event::SizeWarningsReady {
                            request: request.id,
                            result,
                        },
                        &stop,
                    );
                    // Request and any rejected event are destroyed on this worker.
                }
            });
        }
        self.sender = Some(sender.clone());
        sender
    }

    pub(crate) async fn open(
        &mut self,
        id: DesktopRequestId,
        report: SizeWarningReport,
        cancel: Cancellation,
        tasks: &mut JoinSet<()>,
    ) {
        let sender = match &self.sender {
            Some(sender) => sender.clone(),
            None => self.start(tasks),
        };
        if let Err(refused) = sender.try_send(Request { id, report, cancel }) {
            // Await retirement to bound ownership when the queue is full.
            let _ = tokio::task::spawn_blocking(move || drop(refused.into_inner())).await;
            self.outbox
                .send(Event::SizeWarningsReady {
                    request: id,
                    result: Err(error(
                        io::ErrorKind::WouldBlock,
                        "Warning report queue is busy; try again",
                    )),
                })
                .await;
        }
    }

    pub(crate) fn shutdown(&mut self) {
        self.stop.cancel();
        self.sender.take();
    }
}

fn error(kind: io::ErrorKind, message: &str) -> FsError {
    FsError::from_io(
        "size warning report",
        Path::new(""),
        io::Error::new(kind, message),
    )
}

#[cfg(test)]
#[path = "size_warning_requests_tests.rs"]
mod tests;
