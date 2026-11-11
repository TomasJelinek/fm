//! One bounded backend worker retains clipboard ownership and serializes explicit copy actions.
use crate::Outbox;
use midnight_core::{
    Cancellation, ClipboardService, DesktopActionResult, DesktopRequestId, Event, FsError,
};
use std::{io, path::Path, sync::Arc};
use tokio::{sync::mpsc, task::JoinSet};

const MAX_CLIPBOARD_BYTES: usize = 4 * 1024 * 1024;

struct Request {
    id: DesktopRequestId,
    text: String,
}

pub(crate) struct Controller {
    sender: Option<mpsc::Sender<Request>>,
    service: Option<Arc<dyn ClipboardService>>,
    cancel: Cancellation,
    outbox: Outbox,
}

impl Controller {
    pub(crate) fn new(service: Option<Arc<dyn ClipboardService>>, outbox: Outbox) -> Self {
        Self {
            sender: None,
            service,
            cancel: Cancellation::default(),
            outbox,
        }
    }

    fn start_worker(&mut self, tasks: &mut JoinSet<()>) -> mpsc::Sender<Request> {
        let (sender, mut receiver) = mpsc::channel::<Request>(16);
        let service = self.service.take();
        let worker_cancel = self.cancel.clone();
        let worker_outbox = self.outbox.clone();
        tasks.spawn_blocking(move || {
            let mut session = None;
            while let Some(request) = receiver.blocking_recv() {
                if worker_cancel.is_cancelled() {
                    break;
                }
                let result = (|| {
                    if request.text.len() > MAX_CLIPBOARD_BYTES || request.text.contains('\0') {
                        return Err(error(
                            io::ErrorKind::InvalidInput,
                            "Clipboard text exceeds its 4 MiB resource budget or contains NUL.",
                        ));
                    }
                    if session.is_none() {
                        session = Some(
                            service
                                .as_ref()
                                .ok_or_else(|| {
                                    error(
                                        io::ErrorKind::Unsupported,
                                        "Clipboard integration is unavailable.",
                                    )
                                })?
                                .open()?,
                        );
                    }
                    session
                        .as_mut()
                        .expect("clipboard session was initialized")
                        .copy_text(&request.text)?;
                    Ok(DesktopActionResult::ClipboardCopied)
                })();
                let event = Event::DesktopActionFinished {
                    request: request.id,
                    result,
                };
                if !worker_outbox.blocking_send(event, &worker_cancel) {
                    return;
                }
            }
            // Linux clipboard content stays owned until this session is dropped here.
        });
        sender
    }

    pub(crate) async fn copy(
        &mut self,
        id: DesktopRequestId,
        text: String,
        tasks: &mut JoinSet<()>,
    ) {
        // Idle applications must not occupy a blocking worker. Once requested, retain the
        // thread-affine session so Linux clipboard contents remain available to other clients.
        if self.sender.is_none() {
            self.sender = Some(self.start_worker(tasks));
        }
        if self
            .sender
            .as_ref()
            .expect("clipboard worker was started")
            .try_send(Request { id, text })
            .is_err()
        {
            self.outbox
                .send(Event::DesktopActionFinished {
                    request: id,
                    result: Err(error(
                        io::ErrorKind::WouldBlock,
                        "Clipboard queue is busy; try copying again.",
                    )),
                })
                .await;
        }
    }

    pub(crate) fn shutdown(self) {
        drop(self);
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

fn error(kind: io::ErrorKind, message: &str) -> FsError {
    FsError::from_io(
        "copy text to clipboard",
        Path::new(""),
        io::Error::new(kind, message),
    )
}
