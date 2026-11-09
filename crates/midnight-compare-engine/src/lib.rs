//! Bounded, read-only text and image comparison. Every expensive operation runs on a worker.
#![forbid(unsafe_code)]

mod images;
mod text;
mod worker;

#[cfg(test)]
mod capture_tests;

use midnight_core::{FileCompareLaunch, ViewerAppearance, ViewerGeometry};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc::{self, SyncSender},
};

/// Backend-only native source capture. Failure never invokes a fallback opener.
/// The borrowed cancellation flag remains valid for the duration of this call.
pub type NativeCapture = Arc<
    dyn Fn(&std::path::Path, &AtomicBool) -> Result<midnight_plugin_host::CapturedFile, String>
        + Send
        + Sync,
>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ImageMode {
    #[default]
    SideBySide,
    Swipe,
    OnionSkin,
}

#[derive(Clone, Debug)]
pub enum Input {
    Resize(ViewerGeometry, u64),
    Scroll { x: f32, y: f32 },
    Seek(f64),
    Difference { backwards: bool },
    Mode(ImageMode),
    Amount(f64),
    Zoom(f64),
    ActualPixels,
    Fit,
    Close,
}

pub struct Frame {
    pub geometry_revision: u64,
    pub mode: Option<ImageMode>,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub accessible_text: String,
    pub status: String,
    pub image: bool,
    pub position: f64,
}

pub enum Output {
    Frame { frame: Frame, names: [String; 2] },
    Status(String),
}

/// A latest-output slot bounds memory even when the UI is paused.
#[derive(Default)]
pub struct Mailbox(Mutex<Option<Output>>);
impl Mailbox {
    pub fn take(&self) -> Option<Output> {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
    }
    fn put(&self, output: Output) {
        *self.0.lock().unwrap_or_else(|error| error.into_inner()) = Some(output);
    }
}

#[derive(Clone)]
pub struct Service {
    handle: tokio::runtime::Handle,
    capture: NativeCapture,
    active: Arc<AtomicUsize>,
}

pub struct Session {
    sender: SyncSender<Input>,
    cancelled: Arc<AtomicBool>,
    pub mailbox: Arc<Mailbox>,
    handle: tokio::runtime::Handle,
}

impl Session {
    pub fn send(&self, input: Input) -> bool {
        self.sender.try_send(input).is_ok()
    }
    pub fn close(&self) {
        self.cancelled.store(true, Ordering::Release);
        let _ = self.sender.try_send(Input::Close);
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.close();
        let mailbox = std::mem::take(&mut self.mailbox);
        self.handle.spawn_blocking(move || drop(mailbox));
    }
}

struct Permit(Arc<AtomicUsize>);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Service {
    pub fn new(handle: tokio::runtime::Handle) -> Self {
        Self::with_native_capture(
            handle,
            Arc::new(|path, cancel| {
                check_cancel(cancel)?;
                midnight_plugin_host::CapturedFile::capture(path).map_err(|error| error.to_string())
            }),
        )
    }
    /// Install the filesystem's capture authority for both comparison sources.
    /// Invoked only by comparison workers. The callback must retain its provider
    /// for the lifetime of every returned content lease; errors never fall back.
    pub fn with_native_capture(handle: tokio::runtime::Handle, capture: NativeCapture) -> Self {
        Self {
            handle,
            capture,
            active: Arc::new(AtomicUsize::new(0)),
        }
    }
    pub fn open(
        &self,
        launch: FileCompareLaunch,
        appearance: ViewerAppearance,
        geometry: ViewerGeometry,
    ) -> Result<Session, String> {
        self.active
            .try_update(Ordering::AcqRel, Ordering::Relaxed, |count| {
                (count < 2).then_some(count + 1)
            })
            .map_err(|_| "Close a comparator before opening another (maximum two).")?;
        let permit = Permit(self.active.clone());
        let (sender, receiver) = mpsc::sync_channel(32);
        let mailbox = Arc::new(Mailbox::default());
        let cancelled = Arc::new(AtomicBool::new(false));
        let output = mailbox.clone();
        let stop = cancelled.clone();
        let capture = self.capture.clone();
        self.handle.spawn_blocking(move || {
            let _permit = permit;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                worker::run(
                    launch,
                    appearance,
                    geometry,
                    receiver,
                    &output,
                    worker::Capture {
                        factory: &capture,
                        cancel: &stop,
                    },
                )
            }))
            .unwrap_or_else(|_| Err("Comparison stopped after an internal error.".into()));
            if let Err(error) = result
                && !stop.load(Ordering::Acquire)
            {
                output.put(Output::Status(error));
            }
        });
        Ok(Session {
            sender,
            cancelled,
            mailbox,
            handle: self.handle.clone(),
        })
    }
    pub fn retire<T: Send + 'static>(&self, value: T) {
        self.handle.spawn_blocking(move || drop(value));
    }
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Comparison cancelled.".into())
    } else {
        Ok(())
    }
}
