//! Host-owned viewer sessions. UI callbacks only enqueue bounded commands.
#![forbid(unsafe_code)]

mod capture;
mod capture_archive;
mod capture_find;
#[cfg(test)]
mod capture_find_tests;
mod capture_generated;
mod capture_prefetch;
mod capture_tree;
mod catalog_startup;
mod content_resources;
mod document_render;
mod document_view;
pub use content_resources::captured_directory_resources;
#[cfg(test)]
mod capture_tests;
#[cfg(test)]
mod font_catalog_tests;
mod image_geometry;
mod image_input;
mod image_pixels;
mod image_scene;
mod image_startup;
mod image_tiles;
mod image_view;
mod image_worker;
mod input_queue;
mod interaction;
mod layout_handshake;
mod link_slug;
mod mailbox;
mod navigation;
mod office_preview;
#[cfg(test)]
mod opening_retirement_tests;
mod page;
pub use page::role_color;
mod palette;
mod position_ack;
mod preferences;
mod prepared_catalogs;
pub use prepared_catalogs::PreparedCatalogs;
mod raster_budget;
#[cfg(test)]
mod resident_tests;
mod routing;
mod search_cursor;
#[cfg(test)]
mod tests;
mod video_actor;
mod video_discovery;
mod viewport;
mod window_worker;
mod worker;

use midnight_core::{
    ViewerAppearance, ViewerAssociations, ViewerGeometry, ViewerInput, ViewerLaunch, ViewerOutput,
    ViewerPreferenceChange, ViewerPreferences,
};
use midnight_plugin_host::Runtime;
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

/// Backend-only native source capture. Failure never invokes a fallback opener.
/// Nested backend services can observe the same shared close flag for the whole capture.
pub type NativeCapture = Arc<
    dyn Fn(&std::path::Path, &Arc<AtomicBool>) -> Result<midnight_plugin_host::CapturedFile, String>
        + Send
        + Sync,
>;
pub use mailbox::Mailbox;

/// Four admitted windows plus the one source-free image-spare manager.
pub const RESIDENT_WORKERS: usize = 5;

#[derive(Clone)]
pub struct Service {
    handle: tokio::runtime::Handle,
    resident: midnight_work_budget::resident::Spawner,
    capture: NativeCapture,
    runtime: Arc<Mutex<Option<Arc<Runtime>>>>,
    images: Arc<Mutex<Option<Arc<[midnight_image_viewer_host::Provider]>>>>,
    web: Arc<Mutex<Option<Arc<[midnight_web_viewer_host::Provider]>>>>,
    windows: Arc<AtomicUsize>,
    raster_budget: Arc<raster_budget::Budget>,
    fonts: Arc<OnceLock<Arc<[String]>>>,
    /// Installed video support, discovered on demand (see `video_installation`).
    video: Arc<
        video_discovery::Discovered<midnight_media_host::video_installation::VideoInstallation>,
    >,
    catalog_preparation: Arc<catalog_startup::Preparation>,
    image_startup: image_startup::Startup,
    image_policy: Arc<Mutex<Option<image_startup::Policy>>>,
    image_catalog_revision: Arc<AtomicU64>,
}

/// App-owned completion ticket for the source-free image spare only.
/// This does not retain a Service or wait for active foreground image sessions.
pub struct ImageSpareRetirement {
    inner: image_startup::SpareRetirement,
}

impl ImageSpareRetirement {
    /// Permanently stop speculative configuration/refill without waiting for native work.
    pub fn stop_spare(&self) {
        self.inner.stop_spare();
    }

    /// Await spare retirement and owner exit, retaining ownership across shutdown handling.
    /// Owner failure is terminal and does not confirm cleanup. Call outside UI callbacks,
    /// before shutting down the Tokio runtime; keep this ticket through runtime cleanup.
    pub async fn wait(&self) -> Result<(), String> {
        self.inner
            .wait()
            .await
            .map_err(|error| format!("Image spare owner failed: {error:?}; cleanup unconfirmed"))
    }
}

type Catalogs = (
    Arc<Runtime>,
    Arc<[midnight_image_viewer_host::Provider]>,
    Arc<[midnight_web_viewer_host::Provider]>,
);

pub struct WindowSession {
    wakes: Arc<input_queue::Wakes>,
    positions: input_queue::PositionQueue,
    closed: Arc<AtomicBool>,
    pub mailbox: Arc<Mailbox>,
    preferences: Arc<preferences::Preferences>,
    palette: Arc<palette::State>,
    fonts: Arc<OnceLock<Arc<[String]>>>,
}

struct WindowPermit(Arc<AtomicUsize>);
impl Drop for WindowPermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Owns an admitted window until its backend worker starts. If the resident pool drops
/// the worker job without running it (a refused or skipped submission), the slot is
/// released and the opening fails.
struct WindowStart {
    _permit: WindowPermit,
    output: Arc<Mailbox>,
    started: bool,
}

impl Drop for WindowStart {
    fn drop(&mut self) {
        if !self.started {
            let message = "Viewer backend is unavailable.".to_owned();
            self.output.finish_open(Err(message.clone()));
            self.output.put(ViewerOutput::Status(message));
        }
    }
}

impl WindowSession {
    /// Current accepted palette revision, including the opening configuration.
    /// Remains readable after close or worker exit; this is not a paint acknowledgement.
    pub fn palette_revision(&self) -> u64 {
        self.palette.snapshot().0
    }

    /// Accept the application colors only (text, page, selection fill, selection text
    /// and accent); the returned revision is not a paint ACK.
    /// Document family/size and independently accepted viewer preferences stay owned
    /// by their existing controls. Full wake queues retain this bounded latest slot.
    pub fn restore_palette(&self, appearance: &ViewerAppearance) -> Result<u64, String> {
        self.palette.restore(appearance, || {
            self.wakes
                .wake()
                .then_some(())
                .ok_or(std::sync::mpsc::TrySendError::Disconnected(
                    worker::Message::Ready,
                ))
        })
    }

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
            self.wakes.wake();
        }
        Ok(changed)
    }

    /// Restore accepted application state without emitting user-change notifications.
    pub fn restore_preferences(&self, value: Arc<ViewerPreferences>) -> Result<bool, String> {
        let changed = self.preferences.restore(value)?;
        if changed {
            self.wakes.wake();
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
                .enqueue(input, |message| self.wakes.send_input(message))
    }

    pub fn close(&self) {
        self.palette.close();
        // Publishes closed, then wakes the worker through its reserved channel slot.
        self.wakes.close(&self.closed);
        self.mailbox
            .finish_open(Err("Viewer closed before displaying content.".into()));
    }
}

impl Drop for WindowSession {
    fn drop(&mut self) {
        self.close();
    }
}

impl Service {
    /// The application retains the original transient runtime through session,
    /// spare and resident-domain retirement, including refused-work disposal.
    pub fn new(
        handle: tokio::runtime::Handle,
        resident: midnight_work_budget::resident::Spawner,
    ) -> Self {
        Self::with_native_capture(handle, resident, capture::native())
    }

    /// Install the filesystem's source authority before opening any viewer.
    /// Invoked only by viewer workers. The callback must retain its provider for
    /// the lifetime of every returned content lease.
    pub fn with_native_capture(
        handle: tokio::runtime::Handle,
        resident: midnight_work_budget::resident::Spawner,
        capture: NativeCapture,
    ) -> Self {
        Self::initialize(handle, resident, capture, None)
    }

    /// Adopt catalogs discovered from an explicitly selected trusted installation.
    /// Discover the immutable bundle on a backend/setup worker before calling this constructor.
    pub fn with_prepared_catalogs(
        handle: tokio::runtime::Handle,
        resident: midnight_work_budget::resident::Spawner,
        capture: NativeCapture,
        catalogs: PreparedCatalogs,
    ) -> Self {
        Self::initialize(handle, resident, capture, Some(catalogs))
    }

    fn initialize(
        handle: tokio::runtime::Handle,
        resident: midnight_work_budget::resident::Spawner,
        capture: NativeCapture,
        catalogs: Option<PreparedCatalogs>,
    ) -> Self {
        let (runtime, images, web, revision) = match catalogs {
            Some(catalogs) => {
                let (runtime, images, web) = catalogs.into_inner();
                (Some(runtime), Some(images), Some(web), 1)
            }
            None => (None, None, None, 0),
        };
        let image_startup = image_startup::Startup::new(handle.clone(), resident.clone());
        Self {
            handle,
            resident,
            capture,
            runtime: Arc::new(Mutex::new(runtime)),
            images: Arc::new(Mutex::new(images)),
            web: Arc::new(Mutex::new(web)),
            windows: Arc::new(AtomicUsize::new(0)),
            raster_budget: Arc::new(raster_budget::Budget::default()),
            fonts: Arc::new(OnceLock::new()),
            video: Arc::default(),
            catalog_preparation: Arc::default(),
            image_startup,
            image_policy: Arc::default(),
            image_catalog_revision: Arc::new(AtomicU64::new(revision)),
        }
    }

    /// Schedule installation-only preparation before native window creation. This never captures
    /// content or chooses a viewport. One service admits at most one speculative startup task;
    /// ordinary opening uses the same cache locks and reports any discovery failure normally.
    pub fn prepare_catalogs(&self) {
        self.catalog_preparation.start(|| {
            let service = self.clone();
            self.handle.spawn_blocking(move || {
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _ = service.catalogs();
                    service.video_installation();
                }));
            });
        });
    }

    /// Register spare completion before fallible app startup can begin preparation.
    /// The ticket owns completion state, not this Service or its preparation Control.
    pub fn image_spare_retirement(&self) -> ImageSpareRetirement {
        ImageSpareRetirement {
            inner: self.image_startup.spare_retirement(),
        }
    }

    /// Schedule source-free image preparation after effective startup preferences are known.
    /// Callers pass memory-only policy; discovery, validation and child ownership stay off UI.
    pub fn prepare_image_workers(
        &self,
        disabled: Option<Arc<[String]>>,
        associations: Arc<ViewerAssociations>,
    ) {
        let policy = disabled.map(|disabled| image_startup::Policy {
            disabled,
            associations,
            // UI startup policy has no authority over trusted catalog generations.
            catalog_revision: 0,
        });
        {
            let mut current = self
                .image_policy
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if *current == policy {
                return;
            }
            *current = policy.clone();
        }
        // Revoke a prior spare before asynchronously obtaining the replacement catalog.
        self.image_startup.configure(None, Arc::from([]));
        let Some(policy) = policy else {
            return;
        };
        let service = self.clone();
        self.handle.spawn_blocking(move || {
            let catalogs = service.catalogs();
            service.configure_image_spare(policy, catalogs);
            // Startup also prepares what every viewer opening otherwise loads first: the
            // installed video support and the system font database (one-time costs that
            // took seconds on a busy disk).
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                service.video_installation();
                drop(midnight_viewer_render::TextRenderer::default());
                if service.fonts.get().is_none()
                    && let Some(names) = midnight_viewer_render::installed_families(|| false)
                {
                    let _ = service.fonts.set(names);
                }
            }));
        });
    }

    fn configure_image_spare(
        &self,
        mut policy: image_startup::Policy,
        catalogs: Result<Catalogs, String>,
    ) {
        let mut current = self
            .image_policy
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Ok((runtime, images, web)) = catalogs else {
            // Forget a policy whose catalog discovery failed so an explicit later call can prepare again; the explorer prepares once per session.
            if current.as_ref() == Some(&policy) {
                *current = None;
            }
            return;
        };
        let mut identities = std::collections::HashMap::new();
        for id in runtime
            .catalog()
            .iter()
            .map(|provider| &provider.id)
            .chain(images.iter().map(|provider| &provider.id))
            .chain(web.iter().map(|provider| &provider.id))
        {
            *identities.entry(id.as_str()).or_insert(0usize) += 1;
        }
        let eligible = images
            .iter()
            .filter(|provider| identities.get(provider.id.as_str()) == Some(&1))
            .cloned()
            .collect::<Vec<_>>()
            .into();
        if current.as_ref() == Some(&policy) {
            policy.catalog_revision = self.image_catalog_revision.load(Ordering::Acquire);
            self.image_startup.configure(Some(policy), eligible);
        }
    }

    pub fn open(
        &self,
        launch: ViewerLaunch,
        appearance: ViewerAppearance,
        geometry: ViewerGeometry,
    ) -> Result<WindowSession, String> {
        self.open_impl(launch, appearance, geometry, false)
    }

    /// Negotiate each backend-selected layout before opening its content.
    /// The initial geometry is provisional; the frontend must acknowledge every
    /// LayoutSelected with its token and settled canvas, including unchanged sizes.
    /// Existing fixed-viewport consumers should continue to use open().
    pub fn open_with_layout(
        &self,
        launch: ViewerLaunch,
        appearance: ViewerAppearance,
        geometry: ViewerGeometry,
    ) -> Result<WindowSession, String> {
        self.open_impl(launch, appearance, geometry, true)
    }

    fn open_impl(
        &self,
        launch: ViewerLaunch,
        appearance: ViewerAppearance,
        geometry: ViewerGeometry,
        negotiate_layout: bool,
    ) -> Result<WindowSession, String> {
        if let Err(error) = launch.preferences.validate() {
            self.retire(launch);
            return Err(error.to_owned());
        }
        let preferences = Arc::new(preferences::Preferences::new(launch.preferences.clone()));
        let palette = Arc::new(palette::State::new(&appearance));
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
        let start = WindowStart {
            _permit: WindowPermit(self.windows.clone()),
            output: Arc::new(if negotiate_layout {
                Mailbox::with_layout()
            } else {
                Mailbox::default()
            }),
            started: false,
        };
        let (wakes, receiver) = input_queue::Wakes::channel();
        let mailbox = start.output.clone();
        let closed = Arc::new(AtomicBool::new(false));
        let job = window_worker::Job {
            work: window_worker::Work {
                service: self.clone(),
                launch,
                appearance,
                geometry,
                receiver,
                output: mailbox.clone(),
                stop: closed.clone(),
                wakes: wakes.clone(),
                preferences: worker_preferences,
                negotiate_layout,
                palette_lifetime: palette::Lifetime::new(palette.clone()),
            },
            start,
        };
        if let Err(rejected) = self
            .resident
            .spawn(&self.handle, Box::new(move || job.run()))
        {
            let message = format!(
                "Viewer resident worker is unavailable: {:?}.",
                rejected.reason()
            );
            self.retire(rejected.into_work());
            return Err(message);
        }
        Ok(WindowSession {
            wakes,
            positions: Default::default(),
            closed,
            mailbox,
            preferences,
            palette,
            fonts: self.fonts.clone(),
        })
    }

    /// Installed video support for routing, discovered on a backend worker and kept once
    /// found. Discovery validates every file of the video package (726 in a development
    /// installation), which added about 90 ms to every viewer opening of any kind. A failed
    /// discovery (a package being replaced, a busy disk) is tried again by a later opening,
    /// at most once every few seconds (`video_discovery`), instead of leaving video
    /// unavailable until restart. Launching still revalidates the package and pins its code
    /// (`LaunchPins`).
    fn video_installation(
        &self,
    ) -> Option<midnight_media_host::video_installation::VideoInstallation> {
        self.video.get(std::time::Instant::now(), || {
            midnight_plugin_host::installed_executable()
                .ok()
                .and_then(|executable| {
                    midnight_media_host::video_installation::VideoInstallation::discover(
                        &executable,
                    )
                    .ok()
                })
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
        let catalogs =
            prepared_catalogs::load(&executable, runtime.clone(), images.clone(), web.clone())?;
        *runtime = Some(catalogs.0.clone());
        *images = Some(catalogs.1.clone());
        *web = Some(catalogs.2.clone());
        self.image_catalog_revision.fetch_add(1, Ordering::AcqRel);
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
