//! Installed image viewers and fixed-pitch fonts, discovered by the first comparison worker
//! that needs them and kept for the service. Only complete, successful results are kept.
use midnight_image_viewer_host::Provider;
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, TryRecvError},
};

type Discover = Box<dyn Fn() -> Result<Vec<Provider>, String> + Send + Sync>;
type Scan = Box<dyn Fn(&AtomicBool) -> Option<Arc<[String]>> + Send + Sync>;

/// Installed image viewers. A failed discovery is not kept: the next comparison retries
/// it, so one transient error (a package being replaced, a locked file) cannot disable
/// image comparison for the rest of the session.
pub(crate) struct ImageViewers {
    discover: Discover,
    providers: Mutex<Option<Arc<[Provider]>>>,
}

impl ImageViewers {
    /// Packages registered with the installed runtime. Opening a session revalidates them.
    pub(crate) fn installed() -> Self {
        Self::with_discovery(|| {
            let executable =
                midnight_plugin_host::installed_executable().map_err(|error| error.to_string())?;
            midnight_image_viewer_host::discover(&executable).map_err(|error| error.to_string())
        })
    }

    pub(crate) fn with_discovery(
        discover: impl Fn() -> Result<Vec<Provider>, String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            discover: Box::new(discover),
            providers: Mutex::new(None),
        }
    }

    /// The kept providers, or a new discovery announced by `checking`. Two comparisons
    /// that need them at once share one discovery. Backend workers only.
    pub(crate) fn get(&self, checking: impl FnOnce()) -> Result<Arc<[Provider]>, String> {
        let mut providers = self
            .providers
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(providers) = providers.as_ref() {
            return Ok(providers.clone());
        }
        checking();
        let discovered: Arc<[Provider]> = (self.discover)()?.into();
        *providers = Some(discovered.clone());
        Ok(discovered)
    }

    #[cfg(test)]
    pub(crate) fn kept(&self) -> Option<Arc<[Provider]>> {
        self.providers.lock().unwrap().clone()
    }
}

/// Installed fixed-pitch families for the comparison font chooser. The scan maps every
/// installed family and can take seconds on a loaded machine, so it runs on its own
/// `comparator-fonts` thread. A completed catalog is kept for the service: only text
/// comparisons that start while no scan has completed paint before the preferred family is
/// known; every later comparison resolves it before its first frame.
pub(crate) struct FixedPitchFonts {
    scan: Scan,
    families: OnceLock<Arc<[String]>>,
}

impl FixedPitchFonts {
    pub(crate) fn installed() -> Self {
        Self::with_scan(|cancel| {
            midnight_viewer_render::monospace_families(|| cancel.load(Ordering::Acquire))
        })
    }

    pub(crate) fn with_scan(
        scan: impl Fn(&AtomicBool) -> Option<Arc<[String]>> + Send + Sync + 'static,
    ) -> Self {
        Self {
            scan: Box::new(scan),
            families: OnceLock::new(),
        }
    }

    /// A kept catalog is ready at once. Otherwise a scan thread delivers one reply; it is
    /// cancelled with the comparison and keeps a completed catalog for later comparisons.
    pub(crate) fn request(self: &Arc<Self>, cancel: &Arc<AtomicBool>) -> FontCatalog {
        let (sender, receiver) = mpsc::sync_channel(1);
        if let Some(families) = self.families.get() {
            let _ = sender.send(Some(families.clone()));
            return FontCatalog(receiver);
        }
        let reply = sender.clone();
        let fonts = Arc::clone(self);
        let cancel = Arc::clone(cancel);
        let spawned = std::thread::Builder::new()
            .name("comparator-fonts".into())
            .spawn(move || {
                let families = (fonts.scan)(&cancel)
                    .map(|families| fonts.families.get_or_init(|| families).clone());
                let _ = reply.send(families);
            });
        if spawned.is_err() {
            // Without a thread the comparison keeps the generic family and no chooser list.
            let _ = sender.send(Some(Arc::from([])));
        }
        FontCatalog(receiver)
    }
}

/// One catalog request: `Some(families)`, or `None` for a cancelled or failed scan.
pub(crate) struct FontCatalog(Receiver<Option<Arc<[String]>>>);

impl FontCatalog {
    /// The reply once it has arrived. A scan thread that ended without one has failed.
    pub(crate) fn reply(&self) -> Option<Option<Arc<[String]>>> {
        match self.0.try_recv() {
            Ok(reply) => Some(reply),
            Err(TryRecvError::Disconnected) => Some(None),
            Err(TryRecvError::Empty) => None,
        }
    }
}
