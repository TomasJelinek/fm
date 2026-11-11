//! Host-owned viewer sessions. UI callbacks only enqueue bounded commands.
#![forbid(unsafe_code)]

mod capture;
mod capture_archive;
mod capture_generated;
mod capture_tree;
mod catalog_startup;
mod content_resources;
pub use content_resources::captured_directory_resources;
#[cfg(test)]
mod capture_tests;
#[cfg(test)]
mod font_catalog_tests;
mod image_geometry;
mod image_input;
mod image_pixels;
mod image_scene;
mod image_tiles;
mod image_view;
mod image_worker;
mod input_queue;
mod interaction;
mod mailbox;
mod navigation;
#[cfg(test)]
mod opening_retirement_tests;
mod page;
mod position_ack;
mod preferences;
mod raster_budget;
mod routing;
mod search_cursor;
#[cfg(test)]
mod tests;
mod viewport;
mod worker;

use midnight_core::{
    ViewerAppearance, ViewerGeometry, ViewerInput, ViewerLaunch, ViewerOutput,
    ViewerPreferenceChange, ViewerPreferences,
};
use midnight_plugin_host::Runtime;
use std::sync::{
    Arc, Mutex, OnceLock,
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
pub use mailbox::Mailbox;

#[derive(Clone)]
pub struct Service {
    handle: tokio::runtime::Handle,
    capture: NativeCapture,
    runtime: Arc<Mutex<Option<Arc<Runtime>>>>,
    images: Arc<Mutex<Option<Arc<[midnight_image_viewer_host::Provider]>>>>,
    web: Arc<Mutex<Option<Arc<[midnight_web_viewer_host::Provider]>>>>,
    windows: Arc<AtomicUsize>,
    raster_budget: Arc<raster_budget::Budget>,
    fonts: Arc<OnceLock<Arc<[String]>>>,
    catalog_preparation: Arc<catalog_startup::Preparation>,
}

type Catalogs = (
    Arc<Runtime>,
    Arc<[midnight_image_viewer_host::Provider]>,
    Arc<[midnight_web_viewer_host::Provider]>,
);

pub struct WindowSession {
    sender: SyncSender<worker::Message>,
    positions: input_queue::PositionQueue,
    closed: Arc<AtomicBool>,
    pub mailbox: Arc<Mailbox>,
    preferences: Arc<preferences::Preferences>,
    fonts: Arc<OnceLock<Arc<[String]>>>,
}

struct WindowPermit(Arc<AtomicUsize>);
impl Drop for WindowPermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

impl WindowSession {
    /// Cached installed family names. Discovery runs on the session's backend worker.
    /// `None` means discovery is pending or the session closed before it completed.
    pub fn font_families(&self) -> Option<Arc<[String]>> {
        self.fonts.get().cloned()
    }

    /// Bounded config-only acceptance. Never takes a render/decode lock or does I/O.
    /// Callers may save the returned accepted change before hiding the surface.
    pub fn change_preferences(&self, change: &ViewerPreferenceChange) -> Result<bool, String> {
        if self.closed.load(Ordering::Acquire) {
            return Err("Viewer is closed.".into());
        }
        let changed = self.preferences.change(change)?;
        if changed {
            let _ = self.sender.try_send(worker::Message::Ready);
        }
        Ok(changed)
    }

    /// Restore accepted application state without emitting user-change notifications.
    pub fn restore_preferences(&self, value: Arc<ViewerPreferences>) -> Result<bool, String> {
        let changed = self.preferences.restore(value)?;
        if changed {
            let _ = self.sender.try_send(worker::Message::Ready);
        }
        Ok(changed)
    }

    pub fn preferences(&self) -> Arc<ViewerPreferences> {
        self.preferences.snapshot().1
    }

    pub fn send(&self, input: ViewerInput) -> bool {
        !self.closed.load(Ordering::Acquire)
            && self
                .positions
                .enqueue(input, |message| self.sender.try_send(message).is_ok())
    }

    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.mailbox
            .finish_open(Err("Viewer closed before displaying content.".into()));
        let _ = self
            .sender
            .try_send(worker::Message::Input(ViewerInput::Close));
    }
}

impl Drop for WindowSession {
    fn drop(&mut self) {
        self.close();
    }
}

impl Service {
    pub fn new(handle: tokio::runtime::Handle) -> Self {
        Self::with_native_capture(handle, capture::native())
    }

    /// Install the filesystem's source authority before opening any viewer.
    /// Invoked only by viewer workers. The callback must retain its provider for
    /// the lifetime of every returned content lease.
    pub fn with_native_capture(handle: tokio::runtime::Handle, capture: NativeCapture) -> Self {
        Self {
            handle,
            capture,
            runtime: Arc::new(Mutex::new(None)),
            images: Arc::new(Mutex::new(None)),
            web: Arc::new(Mutex::new(None)),
            windows: Arc::new(AtomicUsize::new(0)),
            raster_budget: Arc::new(raster_budget::Budget::default()),
            fonts: Arc::new(OnceLock::new()),
            catalog_preparation: Arc::default(),
        }
    }

    /// Schedule installation-only preparation before native window creation. This never captures
    /// content or chooses a viewport. One service admits at most one speculative startup task;
    /// ordinary opening uses the same cache locks and reports any discovery failure normally.
    pub fn prepare_catalogs(&self) {
        self.catalog_preparation.start(|| {
            let service = self.clone();
            self.handle.spawn_blocking(move || {
                let _ =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| service.catalogs()));
            });
        });
    }

    pub fn open(
        &self,
        launch: ViewerLaunch,
        appearance: ViewerAppearance,
        geometry: ViewerGeometry,
    ) -> Result<WindowSession, String> {
        if let Err(error) = launch.preferences.validate() {
            self.retire(launch);
            return Err(error.to_owned());
        }
        let preferences = Arc::new(preferences::Preferences::new(launch.preferences.clone()));
        let worker_preferences = preferences.clone();
        if self
            .windows
            .try_update(Ordering::AcqRel, Ordering::Relaxed, |count| {
                (count < 4).then_some(count + 1)
            })
            .is_err()
        {
            self.retire(launch);
            return Err("Close a viewer before opening another (maximum four windows).".into());
        }
        let (sender, receiver) = mpsc::sync_channel(64);
        let wakes = sender.clone();
        let mailbox = Arc::new(Mailbox::default());
        let closed = Arc::new(AtomicBool::new(false));
        let service = self.clone();
        let output = mailbox.clone();
        let stop = closed.clone();
        self.handle.spawn_blocking(move || {
            let _permit = WindowPermit(service.windows.clone());
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if service.fonts.get().is_none()
                    && let Some(names) =
                        midnight_viewer_render::installed_families(|| stop.load(Ordering::Acquire))
                {
                    let _ = service.fonts.set(names);
                }
                let (runtime, images, web) = service.catalogs()?;
                worker::run(
                    worker::Services {
                        runtime,
                        images,
                        web,
                        capture: service.capture.clone(),
                        wake: Arc::new(move || wakes.try_send(worker::Message::Ready).is_ok()),
                        preferences: worker_preferences,
                        raster_budget: service.raster_budget.clone(),
                    },
                    launch,
                    appearance,
                    geometry,
                    receiver,
                    &output,
                    &stop,
                )
            }))
            .unwrap_or_else(|_| Err("Viewer backend stopped after an internal error.".into()));
            if let Err(message) = result {
                output.finish_open(Err(message.clone()));
                output.put(ViewerOutput::Status(message));
            }
        });
        Ok(WindowSession {
            sender,
            positions: Default::default(),
            closed,
            mailbox,
            preferences,
            fonts: self.fonts.clone(),
        })
    }

    fn catalogs(&self) -> Result<Catalogs, String> {
        // The existing cache locks also serialize cold initialization across windows.
        // One service starts at most two temporary discovery workers, not two per window.
        let mut runtime = self
            .runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut images = self
            .images
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut web = self.web.lock().unwrap_or_else(|error| error.into_inner());
        if let (Some(runtime), Some(images), Some(web)) = (&*runtime, &*images, &*web) {
            return Ok((runtime.clone(), images.clone(), web.clone()));
        }
        let executable =
            midnight_plugin_host::installed_executable().map_err(|error| error.to_string())?;
        let directory = executable.parent().ok_or("Executable has no directory")?;
        let cached_images = images.clone();
        let cached_web = web.clone();
        let catalogs = catalog_startup::parallel(
            || match runtime.as_ref() {
                Some(runtime) => Ok(runtime.clone()),
                None => Runtime::discover(directory)
                    .map(Arc::new)
                    .map_err(|error| error.to_string()),
            },
            || {
                Ok(cached_images.unwrap_or_else(|| {
                    // Invalid process packages expose no feature. Options retains discovery diagnostics.
                    midnight_image_viewer_host::discover(&executable)
                        .unwrap_or_default()
                        .into()
                }))
            },
            || {
                Ok(cached_web.unwrap_or_else(|| {
                    midnight_web_viewer_host::discover_with_diagnostics(&executable)
                        .map(|discovery| discovery.providers)
                        .unwrap_or_default()
                        .into()
                }))
            },
        )?;
        *runtime = Some(catalogs.0.clone());
        *images = Some(catalogs.1.clone());
        *web = Some(catalogs.2.clone());
        // Routing still sees all three catalogs together, preserving cross-transport duplicate IDs.
        Ok(catalogs)
    }

    /// Large presentation snapshots must also be freed off the event loop.
    pub fn retire<T: Send + 'static>(&self, value: T) {
        let raster_budget = self.raster_budget.clone();
        self.handle.spawn_blocking(move || {
            drop(value);
            raster_budget.reap();
        });
    }
}
