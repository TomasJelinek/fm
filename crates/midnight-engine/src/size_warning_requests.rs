//! Bounded report snapshots; native file ownership never leaves backend workers.
use crate::Outbox;
use crate::resident_tasks::ResidentTasks;
use midnight_core::{
    Cancellation, DesktopRequestId, Event, FsError, GeneratedTextSource, SizeWarningReport,
};
use std::{
    io,
    path::Path,
    sync::{Arc, Mutex},
};
use tokio::sync::mpsc;

const WORKERS: usize = 2;
const PENDING: usize = 8;

enum Content {
    Warnings(SizeWarningReport),
    Diagnostic(Arc<dyn GeneratedTextSource>),
}

struct Request {
    id: DesktopRequestId,
    content: Content,
    cancel: Cancellation,
}

impl Request {
    fn failed(&self, error: FsError) -> Event {
        match self.content {
            Content::Warnings(_) => Event::SizeWarningsReady {
                request: self.id,
                result: Err(error),
            },
            Content::Diagnostic(_) => Event::DiagnosticTextReady {
                request: self.id,
                result: Err(error),
            },
        }
    }

    fn read(&self, cancel: &Cancellation) -> Event {
        match &self.content {
            Content::Warnings(report) => Event::SizeWarningsReady {
                request: self.id,
                result: report.snapshot(cancel),
            },
            Content::Diagnostic(source) => Event::DiagnosticTextReady {
                request: self.id,
                result: crate::diagnostic_text::read(source.as_ref(), cancel),
            },
        }
    }
}

pub(crate) struct Controller {
    resident: midnight_work_budget::resident::Spawner,
    sender: Option<mpsc::Sender<Request>>,
    start_attempted: bool,
    stop: Cancellation,
    outbox: Outbox,
}

impl Controller {
    pub(crate) fn new(resident: midnight_work_budget::resident::Spawner, outbox: Outbox) -> Self {
        Self {
            resident,
            sender: None,
            start_attempted: false,
            stop: Cancellation::default(),
            outbox,
        }
    }

    fn start(&mut self, tasks: &mut ResidentTasks) {
        self.start_attempted = true;
        let (sender, receiver) = mpsc::channel::<Request>(PENDING);
        let receiver = Arc::new(Mutex::new(receiver));
        let mut started = 0;
        for _ in 0..WORKERS {
            let receiver = receiver.clone();
            let stop = self.stop.clone();
            let outbox = self.outbox.clone();
            // Readers live until shutdown drops the sender and occupy resident
            // lanes reserved independently of listing and scan work.
            let work = Box::new(move || {
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
                    let event = if cancel.is_cancelled() {
                        request.failed(error(
                            io::ErrorKind::Interrupted,
                            "Report request cancelled",
                        ))
                    } else {
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            request.read(&cancel)
                        }))
                        .unwrap_or_else(|_| {
                            request.failed(error(io::ErrorKind::Other, "The report worker failed."))
                        })
                    };
                    let event = if cancel.is_cancelled() {
                        request.failed(error(
                            io::ErrorKind::Interrupted,
                            "Report request cancelled",
                        ))
                    } else {
                        event
                    };
                    let delivery = if matches!(request.content, Content::Diagnostic(_)) {
                        &cancel
                    } else {
                        &stop
                    };
                    outbox.blocking_send(event, delivery);
                    // Request and any rejected event are destroyed on this worker.
                }
            });
            if let Some(ticket) = crate::spawn_resident(&self.resident, "report reader", work) {
                tasks.insert(ticket);
                started += 1;
            }
        }
        // With partial admission the shared inbox is empty until every holder,
        // including the refused envelope, is retired.
        self.sender = (started == WORKERS).then_some(sender);
    }

    pub(crate) async fn open(
        &mut self,
        id: DesktopRequestId,
        report: SizeWarningReport,
        cancel: Cancellation,
        tasks: &mut ResidentTasks,
    ) {
        self.enqueue(
            Request {
                id,
                content: Content::Warnings(report),
                cancel,
            },
            tasks,
        )
        .await;
    }

    pub(crate) async fn preview(
        &mut self,
        id: DesktopRequestId,
        content: Arc<dyn GeneratedTextSource>,
        cancel: Cancellation,
        tasks: &mut ResidentTasks,
    ) {
        self.enqueue(
            Request {
                id,
                content: Content::Diagnostic(content),
                cancel,
            },
            tasks,
        )
        .await;
    }

    async fn enqueue(&mut self, request: Request, tasks: &mut ResidentTasks) {
        if !self.start_attempted {
            self.start(tasks);
        }
        let refused = match &self.sender {
            Some(sender) => sender
                .try_send(request)
                .err()
                .map(|refused| refused.into_inner()),
            None => Some(request),
        };
        if let Some(refused) = refused {
            let event = refused.failed(error(
                io::ErrorKind::WouldBlock,
                "Report queue is busy; try again",
            ));
            // Await retirement to bound ownership when the queue is full.
            let _ = tokio::task::spawn_blocking(move || drop(refused)).await;
            self.outbox.send(event).await;
        }
    }

    pub(crate) fn shutdown(&mut self) {
        self.stop.cancel();
        self.sender.take();
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        self.shutdown();
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

#[cfg(test)]
#[path = "diagnostic_request_tests.rs"]
mod diagnostic_tests;
