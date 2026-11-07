//! One bounded, cancellable image actor per viewer window. Owned and reaped by the backend.
use midnight_image_viewer_host::{Frame, ImageSession, Metadata, Provider, Source, Viewport};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::JoinHandle,
    time::Duration,
};

pub(crate) enum Event {
    Opened(Metadata),
    Frame { revision: u64, frame: Frame },
    Failed(String),
}

struct Render {
    revision: u64,
    viewport: Viewport,
}

pub(crate) struct ImageWorker {
    sender: SyncSender<Render>,
    pub events: Receiver<Event>,
    cancel: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ImageWorker {
    pub fn open(provider: Provider, source: Arc<dyn Source>) -> Result<Self, String> {
        let (sender, requests) = mpsc::sync_channel::<Render>(1);
        let (output, events) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let stopped = cancel.clone();
        let thread = std::thread::Builder::new()
            .name("viewer-image".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut session =
                        ImageSession::open(&provider, source, &stopped, Duration::from_secs(30))
                            .map_err(|error| error.to_string())?;
                    if output.try_send(Event::Opened(session.metadata())).is_err() {
                        return Ok(());
                    }
                    while !stopped.load(Ordering::Acquire) {
                        match requests.recv_timeout(Duration::from_millis(10)) {
                            Ok(request) => {
                                let mut frame = session
                                    .render(request.viewport, &stopped, Duration::from_secs(15))
                                    .map_err(|error| error.to_string())?;
                                crate::image_pixels::premultiply(&mut frame.rgba);
                                if stopped.load(Ordering::Acquire)
                                    || output
                                        .try_send(Event::Frame {
                                            revision: request.revision,
                                            frame,
                                        })
                                        .is_err()
                                {
                                    break;
                                }
                            }
                            Err(mpsc::RecvTimeoutError::Timeout) => {}
                            Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        }
                    }
                    Ok::<_, String>(())
                }))
                .unwrap_or_else(|_| Err("Image viewer stopped after an internal error.".into()));
                if let Err(error) = result {
                    let _ = output.try_send(Event::Failed(error));
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            sender,
            events,
            cancel,
            thread: Some(thread),
        })
    }

    pub fn render(&self, revision: u64, viewport: Viewport) -> bool {
        self.sender.try_send(Render { revision, viewport }).is_ok()
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }

    pub fn finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }
}

impl Drop for ImageWorker {
    fn drop(&mut self) {
        self.cancel();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
