//! Shared application content authority. Factories defer all provider work to backend jobs.
use midnight_native_filesystem_host::InstalledProvider;
use midnight_thumbnail_host::service;
use std::{
    collections::VecDeque,
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const MAX_PINS: usize = 16;
/// An unused parent pin is released this long after its last use, so a viewed volume can
/// be ejected.
const PIN_IDLE: Duration = Duration::from_secs(30);

// Thumbnails and comparison need only the borrowed main-content cancellation flag.
pub(crate) type Capture = midnight_compare_engine::NativeCapture;

pub(crate) fn archive(filesystem: Arc<InstalledProvider>) -> midnight_engine::ArchiveCapture {
    Arc::new(move |path, expected, cancel| {
        filesystem
            .capture_matching(path, expected, &|| cancel.is_cancelled())
            .map_err(|failure| {
                let error = if cancel.is_cancelled() {
                    io::ErrorKind::Interrupted.into()
                } else {
                    io::Error::other(format!("Native filesystem: {failure}"))
                };
                midnight_core::FsError::from_io("capture archive", path, error)
            })
    })
}

pub(crate) fn factory(filesystem: Arc<InstalledProvider>) -> Capture {
    Arc::new(move |path, cancel| {
        filesystem
            .capture(path, &|| cancel.load(Ordering::Acquire))
            .map_err(|failure| format!("Native filesystem: {failure}"))
    })
}

/// Parent-directory pins shared by sibling viewer files: at most `MAX_PINS`, each released
/// `PIN_IDLE` after its last use by a reaper armed for the earliest expiry. Pins are
/// dropped only on backend workers, never on the UI: by `pin` (called on a worker), by
/// the reaper on a blocking worker, and, when the last owner drops the set (the UI thread
/// at exit), by a blocking worker it hands the remaining pins to.
pub(crate) struct DirectoryPins<T: Send + 'static> {
    entries: Mutex<VecDeque<(PathBuf, T, Instant)>>,
    reaper: AtomicBool,
    runtime: tokio::runtime::Handle,
    idle: Duration,
}

impl<T: Send + 'static> Drop for DirectoryPins<T> {
    fn drop(&mut self) {
        let entries = std::mem::take(
            self.entries
                .get_mut()
                .unwrap_or_else(|error| error.into_inner()),
        );
        if !entries.is_empty() {
            // Closing a handle can block (an unreachable share); never on the owner's
            // thread. A runtime that is already shutting down drops them here instead.
            drop(self.runtime.spawn_blocking(move || drop(entries)));
        }
    }
}

impl<T: Clone + Send + Sync + 'static> DirectoryPins<T> {
    pub(crate) fn new(runtime: tokio::runtime::Handle) -> Arc<Self> {
        Self::with_idle(runtime, PIN_IDLE)
    }

    fn with_idle(runtime: tokio::runtime::Handle, idle: Duration) -> Arc<Self> {
        Arc::new(Self {
            entries: Mutex::new(VecDeque::new()),
            reaper: AtomicBool::new(false),
            runtime,
            idle,
        })
    }

    /// Reuse a cached pin that `valid` accepts, or keep a freshly `prepare`d one.
    /// Call on a backend worker: expired pins are released here.
    pub(crate) fn pin(
        self: &Arc<Self>,
        parent: &Path,
        valid: impl FnOnce(&T) -> bool,
        prepare: impl FnOnce() -> Option<T>,
    ) -> Option<T> {
        let now = Instant::now();
        let (cached, expired) = {
            let mut entries = self.lock();
            let expired = take_expired(&mut entries, now, self.idle);
            let cached =
                entries
                    .iter_mut()
                    .find(|(key, _, _)| key == parent)
                    .map(|(_, pin, used)| {
                        *used = now;
                        pin.clone()
                    });
            (cached, expired)
        };
        drop(expired);
        if let Some(pin) = cached.filter(|pin| valid(pin)) {
            self.schedule_reaper();
            return Some(pin);
        }
        let prepared = prepare()?;
        let evicted = {
            let mut entries = self.lock();
            let mut evicted = Vec::new();
            if let Some(index) = entries.iter().position(|(key, _, _)| key == parent) {
                evicted.extend(entries.remove(index));
            }
            entries.push_back((parent.to_path_buf(), prepared.clone(), now));
            while entries.len() > MAX_PINS {
                evicted.extend(entries.pop_front());
            }
            evicted
        };
        drop(evicted);
        self.schedule_reaper();
        Some(prepared)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<(PathBuf, T, Instant)>> {
        self.entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    /// Release idle pins even when no further file is viewed: one reaper at a time waits
    /// for the earliest expiry, releases what expired and re-arms for the next one. The
    /// flag is set and cleared under the entries lock, so a pin added while the reaper
    /// finishes is always covered by it or by a new reaper. The task holds no pin.
    fn schedule_reaper(self: &Arc<Self>) {
        let delay = {
            let entries = self.lock();
            let Some(delay) = next_expiry(&entries, Instant::now(), self.idle) else {
                return;
            };
            if self.reaper.swap(true, Ordering::AcqRel) {
                return;
            }
            delay
        };
        let pins: Weak<Self> = Arc::downgrade(self);
        self.runtime.spawn(async move {
            tokio::time::sleep(delay).await;
            let _ = tokio::task::spawn_blocking(move || {
                let Some(pins) = pins.upgrade() else {
                    return;
                };
                let expired = {
                    let mut entries = pins.lock();
                    pins.reaper.store(false, Ordering::Release);
                    take_expired(&mut entries, Instant::now(), pins.idle)
                };
                drop(expired);
                pins.schedule_reaper();
            })
            .await;
        });
    }
}

/// How long until the least recently used pin expires, or `None` without pins.
fn next_expiry<T>(
    entries: &VecDeque<(PathBuf, T, Instant)>,
    now: Instant,
    idle: Duration,
) -> Option<Duration> {
    let last_use = entries.iter().map(|(_, _, used)| *used).min()?;
    Some(idle.saturating_sub(now.saturating_duration_since(last_use)))
}

fn take_expired<T>(
    entries: &mut VecDeque<(PathBuf, T, Instant)>,
    now: Instant,
    idle: Duration,
) -> Vec<T> {
    let mut expired = Vec::new();
    let mut index = 0;
    while index < entries.len() {
        if now.saturating_duration_since(entries[index].2) >= idle {
            expired.extend(entries.remove(index).map(|(_, pin, _)| pin));
        } else {
            index += 1;
        }
    }
    expired
}

/// Metadata-only external-compare capture through the same installed cache.
/// The Engine front door arms each successful return before checking cancellation.
pub(crate) fn external_comparison_factory(
    filesystem: Arc<InstalledProvider>,
) -> midnight_engine::ExternalCompareNativeCapture {
    Arc::new(move |path, cancel| {
        filesystem
            .capture_checked(path, &|| cancel.is_cancelled())
            .map_err(|failure| external_capture_error(path, cancel, failure))
    })
}

fn external_capture_error(
    path: &Path,
    cancel: &midnight_core::Cancellation,
    failure: midnight_native_filesystem_host::CaptureFailure,
) -> midnight_engine::ExternalCompareCaptureError {
    let error = if cancel.is_cancelled() {
        io::ErrorKind::Interrupted.into()
    } else {
        io::Error::other(format!("Native filesystem: {}", failure.error))
    };
    midnight_engine::ExternalCompareCaptureError {
        error: midnight_core::FsError::from_io("capture comparison", path, error),
        cleanup_unconfirmed: failure.cleanup_unconfirmed,
    }
}

/// Viewer-only asset authority. Main content always comes from the installed filesystem plugin.
/// Parent directory pins are shared across prefetched panel files (see `DirectoryPins`).
/// A viewer keeps reading a file whose change time alone moved after capture (VIEW-05).
pub(crate) fn factory_with_resources(
    filesystem: Arc<InstalledProvider>,
    directories: Arc<dyn midnight_core::TransferService>,
    runtime: &tokio::runtime::Handle,
) -> midnight_viewer_engine::NativeCapture {
    type Directory = Arc<dyn midnight_core::TransferDirectory>;
    let pins = DirectoryPins::<Directory>::new(runtime.clone());
    Arc::new(move |path, cancel| {
        let main = filesystem
            .capture_with(
                path,
                midnight_plugin_host::RevisionPolicy::AcceptChangeTimeOnly,
                &|| cancel.load(Ordering::Acquire),
            )
            .map_err(|failure| format!("Native filesystem: {failure}"))?;
        if cancel.load(Ordering::Acquire) {
            return Err("Viewer closed".into());
        }
        let Some(parent) = path.parent() else {
            return Ok(main);
        };
        // This is the original selected native parent, never CapturedFile.path() or a display name.
        let cancelled = midnight_core::Cancellation::observing(cancel.clone());
        let directory = pins.pin(
            parent,
            |directory| directory.revalidate(&cancelled).is_ok(),
            || {
                directories
                    .prepare_checksum_directory(parent, &cancelled)
                    .ok()
            },
        );
        if cancel.load(Ordering::Acquire) {
            return Err("Viewer closed".into());
        }
        match directory.and_then(|directory| {
            midnight_viewer_engine::captured_directory_resources(directory, main.clone()).ok()
        }) {
            Some(resources) => Ok(main.with_resources(resources)),
            // A denied/unavailable scope never gains a native path fallback. The main document
            // remains readable; the web broker reports unavailable relative resources on lookup.
            None => Ok(main),
        }
    })
}

pub(crate) fn thumbnail(capture: Capture, path: PathBuf, confirmed_ssd: bool) -> service::Capture {
    Arc::new(move |cancel| {
        let file = capture(&path, cancel).map_err(|message| {
            if cancel.load(Ordering::Acquire) {
                io::ErrorKind::Interrupted.into()
            } else {
                io::Error::other(message)
            }
        })?;
        Ok(service::captured_source(file, confirmed_ssd))
    })
}

#[cfg(test)]
#[path = "native_content_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "native_pin_tests.rs"]
mod pin_tests;

#[cfg(test)]
#[path = "native_archive_tests.rs"]
mod archive_tests;

#[cfg(test)]
#[path = "native_external_compare_tests.rs"]
mod external_compare_tests;
