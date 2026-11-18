//! Installed image viewers and fixed-pitch fonts, discovered by the first comparison worker
//! that needs them and kept for the service. Only complete, successful results are kept.
use midnight_image_viewer_host::Provider;
use std::{
    cell::RefCell,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
};

type Discover = Box<dyn Fn() -> Result<Vec<Provider>, String> + Send + Sync>;
/// Scans the installed fixed-pitch families; `None` when the cancellation predicate ended it.
type Scan = Box<dyn Fn(&dyn Fn() -> bool) -> Option<Arc<[String]>> + Send + Sync>;
/// The catalog name of the first candidate the scan would list, without scanning the rest.
type Check = Box<dyn Fn(&[&str], &dyn Fn() -> bool) -> Option<String> + Send + Sync>;
/// Receives one scan's catalog, or `None` when it was cancelled, failed or panicked.
/// Called on the scan thread only.
pub(crate) type Deliver = Box<dyn FnOnce(Option<Arc<[String]>>) + Send>;

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

/// Installed fixed-pitch families for the comparison font chooser. The scan loads a face
/// of every installed family and can take seconds on a loaded machine, so it runs on its
/// own `comparator-fonts` thread, at most one at a time for the service: requests that
/// arrive while it runs wait for the same scan, from either comparison window. A
/// completed catalog is kept for the service. Until one is kept, a comparison paints in
/// the family `resolve` finds by checking at most two families.
pub(crate) struct FixedPitchFonts {
    scan: Scan,
    check: Check,
    state: Mutex<Scanning>,
    #[cfg(test)]
    pub(crate) unspawnable: bool,
}

#[derive(Default)]
struct Scanning {
    families: Option<Arc<[String]>>,
    /// The requests waiting for the scan in flight; `None` while no scan owns them.
    waiting: Option<Vec<Waiter>>,
}

struct Waiter {
    cancel: Arc<AtomicBool>,
    deliver: Deliver,
}

impl FixedPitchFonts {
    pub(crate) fn installed() -> Self {
        Self::with_scan(|cancelled| midnight_viewer_render::monospace_families(cancelled))
    }

    pub(crate) fn with_scan(
        scan: impl Fn(&dyn Fn() -> bool) -> Option<Arc<[String]>> + Send + Sync + 'static,
    ) -> Self {
        Self {
            scan: Box::new(scan),
            check: Box::new(|candidates, cancelled| {
                midnight_viewer_render::fixed_pitch_family(candidates, cancelled)
            }),
            state: Mutex::default(),
            #[cfg(test)]
            unspawnable: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn set_check(
        &mut self,
        check: impl Fn(&[&str], &dyn Fn() -> bool) -> Option<String> + Send + Sync + 'static,
    ) {
        self.check = Box::new(check);
    }

    fn state(&self) -> MutexGuard<'_, Scanning> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// The kept catalog, ready at once. Otherwise `deliver` receives the reply of the scan
    /// in flight, or of one started for it; the scan stops when every comparison waiting
    /// for it is cancelled. Without a thread for the scan the comparison gets an empty
    /// catalog at once, which keeps the generic family and no chooser list; `deliver` is
    /// then never called, so it can never wait for this caller.
    pub(crate) fn request(
        self: &Arc<Self>,
        cancel: &Arc<AtomicBool>,
        deliver: Deliver,
    ) -> Option<Arc<[String]>> {
        let mut state = self.state();
        if let Some(families) = &state.families {
            return Some(families.clone());
        }
        let waiter = Waiter {
            cancel: Arc::clone(cancel),
            deliver,
        };
        if let Some(waiting) = &mut state.waiting {
            waiting.push(waiter);
            return None;
        }
        state.waiting = Some(vec![waiter]);
        if self.spawn_scan().is_err() {
            state.waiting = None;
            return Some(Arc::from([]));
        }
        None
    }

    fn spawn_scan(self: &Arc<Self>) -> std::io::Result<()> {
        #[cfg(test)]
        if self.unspawnable {
            return Err(std::io::Error::other("no scan thread in this test"));
        }
        let fonts = Arc::clone(self);
        std::thread::Builder::new()
            .name("comparator-fonts".into())
            .spawn(move || fonts.scan_and_deliver())
            .map(drop)
    }

    /// Runs on the scan thread. Every waiter is answered once, also when the scan panics.
    fn scan_and_deliver(&self) {
        let mut finish = Finish {
            fonts: self,
            families: None,
            detached: RefCell::new(None),
        };
        finish.families = (self.scan)(&|| self.abandoned(&finish.detached));
    }

    /// Whether every waiting comparison was cancelled. The scan then gives its waiters up
    /// at once, so a later request starts a new scan instead of joining this ending one.
    fn abandoned(&self, detached: &RefCell<Option<Vec<Waiter>>>) -> bool {
        let mut state = self.state();
        if detached.borrow().is_some() {
            return true;
        }
        let cancelled = state.waiting.as_ref().is_none_or(|waiting| {
            waiting
                .iter()
                .all(|waiter| waiter.cancel.load(Ordering::Acquire))
        });
        if cancelled {
            *detached.borrow_mut() = Some(state.waiting.take().unwrap_or_default());
        }
        cancelled
    }

    /// The family a text comparison paints in before its catalog is known: the preferred
    /// family when the catalog would list it, else Consolas, else generic monospace, as the
    /// catalog resolves them. A kept catalog answers directly; otherwise one check of at
    /// most these two families runs on the calling worker.
    pub(crate) fn resolve(&self, preferred: &str, cancel: &AtomicBool) -> String {
        if let Some(families) = self.state().families.clone() {
            return crate::text::resolve_family(&families, preferred);
        }
        if preferred == "monospace" {
            return preferred.into();
        }
        let candidates: &[&str] = if preferred.eq_ignore_ascii_case("Consolas") {
            &[preferred]
        } else {
            &[preferred, "Consolas"]
        };
        (self.check)(candidates, &|| cancel.load(Ordering::Acquire))
            .unwrap_or_else(|| "monospace".into())
    }
}

/// Keeps a completed catalog and answers the scan's waiters when the scan ends, also by
/// unwinding from a panic (then with `None`).
struct Finish<'a> {
    fonts: &'a FixedPitchFonts,
    families: Option<Arc<[String]>>,
    /// The waiters this scan gave up when they were all cancelled.
    detached: RefCell<Option<Vec<Waiter>>>,
}

impl Drop for Finish<'_> {
    fn drop(&mut self) {
        let mut waiting = self.detached.get_mut().take();
        {
            let mut state = self.fonts.state();
            if waiting.is_none() {
                waiting = state.waiting.take();
            }
            if let Some(families) = &self.families
                && state.families.is_none()
            {
                state.families = Some(families.clone());
            }
        }
        // Outside the lock: a delivery may wait for its comparison to take an input.
        for waiter in waiting.into_iter().flatten() {
            (waiter.deliver)(self.families.clone());
        }
    }
}
