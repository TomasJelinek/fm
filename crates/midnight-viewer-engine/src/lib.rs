//! Host-owned viewer sessions. UI callbacks only enqueue bounded commands.
#![forbid(unsafe_code)]

mod capture;
mod capture_archive;
mod interaction;
mod navigation;
mod page;
mod search_cursor;
#[cfg(test)]
mod tests;
mod worker;

use midnight_core::{ViewerAppearance, ViewerGeometry, ViewerInput, ViewerLaunch, ViewerOutput};
use midnight_plugin_host::Runtime;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc::{self, SyncSender},
};

/// Latest-frame mailbox: a slow UI cannot accumulate raster images.
pub struct Mailbox {
    output: Mutex<Vec<ViewerOutput>>,
}

impl Mailbox {
    pub fn take(&self) -> Vec<ViewerOutput> {
        std::mem::take(&mut *self.output.lock().unwrap_or_else(|e| e.into_inner()))
    }

    fn put(&self, output: ViewerOutput) {
        let mut pending = self.output.lock().unwrap_or_else(|e| e.into_inner());
        // Release replaced buffers here, on the backend worker.
        match &output {
            ViewerOutput::Frame(_) => pending.retain(|old| !matches!(old, ViewerOutput::Frame(_))),
            ViewerOutput::Status(_) => {
                pending.retain(|old| !matches!(old, ViewerOutput::Status(_)))
            }
            ViewerOutput::Closed => pending.clear(),
            ViewerOutput::CopyText(_) => {
                pending.retain(|old| !matches!(old, ViewerOutput::CopyText(_)))
            }
        }
        if pending.len() < 8 {
            pending.push(output);
        }
    }
}

#[derive(Clone)]
pub struct Service {
    handle: tokio::runtime::Handle,
    runtime: Arc<Mutex<Option<Arc<Runtime>>>>,
    windows: Arc<AtomicUsize>,
}

pub struct WindowSession {
    sender: SyncSender<ViewerInput>,
    closed: Arc<AtomicBool>,
    pub mailbox: Arc<Mailbox>,
}

struct WindowPermit(Arc<AtomicUsize>);
impl Drop for WindowPermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

impl WindowSession {
    pub fn send(&self, input: ViewerInput) -> bool {
        self.sender.try_send(input).is_ok()
    }

    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        let _ = self.sender.try_send(ViewerInput::Close);
    }
}

impl Drop for WindowSession {
    fn drop(&mut self) {
        self.close();
    }
}

impl Service {
    pub fn new(handle: tokio::runtime::Handle) -> Self {
        Self {
            handle,
            runtime: Arc::new(Mutex::new(None)),
            windows: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn open(
        &self,
        launch: ViewerLaunch,
        appearance: ViewerAppearance,
        geometry: ViewerGeometry,
    ) -> Result<WindowSession, String> {
        self.windows
            .try_update(Ordering::AcqRel, Ordering::Relaxed, |count| {
                (count < 4).then_some(count + 1)
            })
            .map_err(|_| {
                "Close a viewer before opening another (maximum four windows).".to_owned()
            })?;
        let (sender, receiver) = mpsc::sync_channel(64);
        let mailbox = Arc::new(Mailbox {
            output: Mutex::new(Vec::new()),
        });
        let closed = Arc::new(AtomicBool::new(false));
        let service = self.clone();
        let output = mailbox.clone();
        let stop = closed.clone();
        self.handle.spawn_blocking(move || {
            let _permit = WindowPermit(service.windows.clone());
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let runtime = {
                    let mut loaded = service.runtime.lock().unwrap_or_else(|e| e.into_inner());
                    if loaded.is_none() {
                        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
                        let directory = executable.parent().ok_or("Executable has no directory")?;
                        *loaded = Some(Arc::new(
                            Runtime::discover(directory).map_err(|e| e.to_string())?,
                        ));
                    }
                    loaded.as_ref().unwrap().clone()
                };
                worker::run(
                    runtime, launch, appearance, geometry, receiver, &output, &stop,
                )
            }))
            .unwrap_or_else(|_| Err("Viewer backend stopped after an internal error.".into()));
            if let Err(message) = result {
                output.put(ViewerOutput::Status(message));
            }
        });
        Ok(WindowSession {
            sender,
            closed,
            mailbox,
        })
    }

    /// Large presentation snapshots must also be freed off the event loop.
    pub fn retire<T: Send + 'static>(&self, value: T) {
        self.handle.spawn_blocking(move || drop(value));
    }
}
