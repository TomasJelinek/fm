//! Bounded pixel conversion between backend output and native presentation.
//! Slint images remain UI-owned; their thread-safe pixel buffers are prepared here.
use midnight_core::{ViewerFrame, ViewerOutput};
use midnight_viewer_engine::Mailbox;
use slint::{Rgba8Pixel, SharedPixelBuffer};
use std::{
    sync::{Arc, Condvar, Mutex, MutexGuard},
    time::{Duration, Instant},
};

const RETAINED_BYTES: u64 = 6 * 1024 * 1024 * 4;
const LEGACY_BYTES: u64 = 16 * 1024 * 1024 * 4;
const WAKE_RETRY: Duration = Duration::from_millis(50);
type Notify = Arc<dyn Fn() -> bool + Send + Sync>;

#[derive(Clone)]
pub(super) struct PreparedPixels {
    pub frame_revision: u64,
    pub body: SharedPixelBuffer<Rgba8Pixel>,
    pub gutter: Option<SharedPixelBuffer<Rgba8Pixel>>,
}

pub(super) struct PreparedBatch {
    pub outputs: Vec<ViewerOutput>,
    pub pixels: Result<Option<PreparedPixels>, String>,
}

#[derive(Default)]
struct State {
    stopped: bool,
    dirty: bool,
    batch: Option<PreparedBatch>,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    ready: Condvar,
}

impl Shared {
    fn signal(&self) -> bool {
        let mut state = lock(&self.state);
        if state.stopped {
            return false;
        }
        state.dirty = true;
        drop(state);
        self.ready.notify_one();
        true
    }
}

/// One prepared delivery waits for the UI. Until it is consumed, the original
/// mailbox retains its existing bounded coalescing and ordering policy.
pub(super) struct Preparation {
    shared: Arc<Shared>,
}

impl Preparation {
    pub fn new(mailbox: Arc<Mailbox>, notify: Notify) -> Result<Self, String> {
        let source = mailbox.clone();
        let mut cache = Cache::default();
        let preparation = Self::start(
            move || source.take(),
            move |outputs| batch(outputs, &mut cache),
            notify,
        )?;
        let shared = Arc::downgrade(&preparation.shared);
        mailbox.notify_on_ready(Arc::new(move || {
            if let Some(shared) = shared.upgrade() {
                shared.signal()
            } else {
                false
            }
        }));
        Ok(preparation)
    }

    fn start(
        mut drain: impl FnMut() -> Vec<ViewerOutput> + Send + 'static,
        mut prepare: impl FnMut(Vec<ViewerOutput>) -> PreparedBatch + Send + 'static,
        notify: Notify,
    ) -> Result<Self, String> {
        let shared = Arc::new(Shared::default());
        lock(&shared.state).dirty = true;
        let worker = shared.clone();
        std::thread::Builder::new()
            .name("viewer-pixels".into())
            .spawn(move || {
                loop {
                    let mut state = lock(&worker.state);
                    while !state.stopped && !state.dirty {
                        state = worker
                            .ready
                            .wait(state)
                            .unwrap_or_else(|error| error.into_inner());
                    }
                    if state.stopped {
                        let abandoned = state.batch.take();
                        drop(state);
                        drop(abandoned);
                        drop(drain());
                        return;
                    }
                    state.dirty = false;
                    drop(state);
                    let outputs = drain();
                    if outputs.is_empty() {
                        continue;
                    }
                    let terminal = outputs
                        .iter()
                        .any(|output| matches!(output, ViewerOutput::Closed));
                    let prepared = prepare(outputs);
                    let mut state = lock(&worker.state);
                    if state.stopped {
                        drop(state);
                        drop(prepared);
                        drop(drain());
                        return;
                    }
                    debug_assert!(state.batch.is_none());
                    state.batch = Some(prepared);
                    drop(state);
                    if !deliver(&worker, &notify) {
                        drop(drain());
                        return;
                    }
                    if terminal {
                        return;
                    }
                }
            })
            .map_err(|error| format!("Cannot start viewer pixel preparation: {error}"))?;
        Ok(Self { shared })
    }

    pub fn take(&self) -> Option<PreparedBatch> {
        let batch = lock(&self.shared.state).batch.take();
        self.shared.ready.notify_one();
        batch
    }

    /// Closing only signals cancellation. Pending pixels and source output are
    /// reclaimed by the worker; joining or dropping a large batch never blocks UI.
    pub fn stop(&self) {
        lock(&self.shared.state).stopped = true;
        self.shared.ready.notify_one();
    }
}

impl Drop for Preparation {
    fn drop(&mut self) {
        self.stop();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

fn deliver(shared: &Shared, notify: &Notify) -> bool {
    let mut notified = false;
    let mut retry_at = Instant::now();
    let mut state = lock(&shared.state);
    loop {
        if state.stopped {
            let abandoned = state.batch.take();
            drop(state);
            drop(abandoned);
            return false;
        }
        if state.batch.is_none() {
            return true;
        }
        if !notified && Instant::now() >= retry_at {
            drop(state);
            notified = notify();
            retry_at = Instant::now() + WAKE_RETRY;
            state = lock(&shared.state);
            continue;
        }
        state = if notified {
            shared
                .ready
                .wait(state)
                .unwrap_or_else(|error| error.into_inner())
        } else {
            shared
                .ready
                .wait_timeout(state, retry_at.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|error| error.into_inner())
                .0
        };
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Key {
    generation: u64,
    surface: u64,
    pixel: u64,
    size: [u32; 2],
    viewport: [u32; 2],
    gutter: Option<[u32; 2]>,
}

#[derive(Default)]
struct Cache {
    retained: Option<(Key, PreparedPixels)>,
}

impl Cache {
    fn prepare(&mut self, frame: &ViewerFrame) -> Result<Option<PreparedPixels>, String> {
        if frame.tiles.is_some() {
            self.retained = None;
            return Ok(None);
        }
        validate(frame)?;
        let key = frame.raster.as_ref().map(|info| Key {
            generation: frame.generation,
            surface: info.surface.0,
            pixel: info.pixel_id,
            size: [frame.width, frame.height],
            viewport: info.viewport,
            gutter: info
                .gutter
                .as_ref()
                .map(|gutter| [gutter.width, gutter.height]),
        });
        if let Some((previous, pixels)) = &self.retained
            && Some(*previous) == key
        {
            return Ok(Some(PreparedPixels {
                frame_revision: frame.revision,
                ..pixels.clone()
            }));
        }
        let pixels = PreparedPixels {
            frame_revision: frame.revision,
            body: SharedPixelBuffer::clone_from_slice(&frame.pixels, frame.width, frame.height),
            gutter: frame
                .raster
                .as_ref()
                .and_then(|info| info.gutter.as_ref())
                .map(|gutter| {
                    SharedPixelBuffer::clone_from_slice(&gutter.pixels, gutter.width, gutter.height)
                }),
        };
        // Cache only converted buffers. Retaining a source Arc would consume the
        // backend's bounded retained-raster reservation after that frame retired.
        self.retained = key.map(|key| (key, pixels.clone()));
        Ok(Some(pixels))
    }
}

fn batch(outputs: Vec<ViewerOutput>, cache: &mut Cache) -> PreparedBatch {
    let pixels = outputs
        .iter()
        .find_map(|output| match output {
            ViewerOutput::Frame(frame) => Some(cache.prepare(frame)),
            _ => None,
        })
        .unwrap_or(Ok(None));
    PreparedBatch { outputs, pixels }
}

/// Shared validation/conversion for isolated presentation fixtures. Production
/// invokes the cached variant exclusively on the preparation worker.
#[cfg(test)]
pub(super) fn prepare(frame: &ViewerFrame) -> Result<Option<PreparedPixels>, String> {
    Cache::default().prepare(frame)
}

fn validate(frame: &ViewerFrame) -> Result<(), String> {
    let bytes = pixel_bytes(frame.width, frame.height)
        .filter(|bytes| *bytes == frame.pixels.len() as u64)
        .ok_or("Invalid viewer pixel buffer.")?;
    let Some(info) = &frame.raster else {
        return (bytes <= LEGACY_BYTES)
            .then_some(())
            .ok_or_else(|| "Viewer pixel buffer exceeds its budget.".into());
    };
    if info.surface.0 == 0 || info.pixel_id == 0 || info.viewport.contains(&0) {
        return Err("Invalid retained viewer surface.".into());
    }
    let gutter = if let Some(gutter) = &info.gutter {
        if gutter.width >= info.viewport[0] || gutter.height != frame.height {
            return Err("Invalid retained viewer gutter.".into());
        }
        pixel_bytes(gutter.width, gutter.height)
            .filter(|bytes| *bytes == gutter.pixels.len() as u64)
            .ok_or("Invalid retained viewer gutter pixels.")?
    } else {
        0
    };
    if bytes
        .checked_add(gutter)
        .is_none_or(|total| total > RETAINED_BYTES)
    {
        return Err("Retained viewer pixels exceed their budget.".into());
    }
    Ok(())
}

fn pixel_bytes(width: u32, height: u32) -> Option<u64> {
    if width == 0 || height == 0 {
        return None;
    }
    u64::from(width)
        .checked_mul(u64::from(height))?
        .checked_mul(4)
}

#[cfg(test)]
#[path = "pixel_preparation_tests.rs"]
mod tests;
