//! Bounded pixel conversion between backend output and native presentation.
//! Slint images remain UI-owned; their thread-safe pixel buffers are prepared here.
use midnight_core::{ViewerFrame, ViewerOutput};
use midnight_viewer_engine::Mailbox;
use slint::{Rgba8Pixel, SharedPixelBuffer};
use std::{
    sync::{Arc, Condvar, Mutex, MutexGuard},
    time::{Duration, Instant},
};

/// Debug-only, opt-in first-frame timestamps. Disabled instances own no allocation.
#[derive(Clone, Default)]
pub(super) struct OpeningTrace {
    #[cfg(debug_assertions)]
    state: Option<Arc<Mutex<opening_trace::State>>>,
}

impl OpeningTrace {
    pub fn new() -> Self {
        #[cfg(debug_assertions)]
        {
            use std::sync::{
                OnceLock,
                atomic::{AtomicU64, Ordering},
            };
            static ENABLED: OnceLock<bool> = OnceLock::new();
            static ORIGIN: OnceLock<Instant> = OnceLock::new();
            static OPENING: AtomicU64 = AtomicU64::new(1);
            if *ENABLED.get_or_init(|| {
                std::env::var_os("MIDNIGHT_VIEWER_OPEN_TRACE").is_some_and(|value| value == "1")
            }) {
                let trace = Self {
                    state: Some(Arc::new(Mutex::new(opening_trace::State::new(
                        *ORIGIN.get_or_init(Instant::now),
                        OPENING.fetch_add(1, Ordering::Relaxed),
                    )))),
                };
                trace.mark(1, 0, 0, 0);
                return trace;
            }
        }
        Self::default()
    }

    pub fn active(&self) -> bool {
        #[cfg(debug_assertions)]
        if let Some(state) = &self.state {
            return !lock(state).finished;
        }
        false
    }

    pub fn mark(&self, phase: u8, generation: u64, revision: u64, pending: u8) {
        #[cfg(debug_assertions)]
        if let Some(state) = &self.state {
            lock(state).mark(phase, generation, revision, pending);
        }
        let _ = (phase, generation, revision, pending);
    }

    pub fn outputs(&self, phase: u8, outputs: &[ViewerOutput]) {
        if !self.active() {
            return;
        }
        let frame = outputs.iter().find_map(|output| match output {
            ViewerOutput::Frame(frame) => Some((frame.generation, frame.revision)),
            _ => None,
        });
        let (generation, revision) = frame.unwrap_or((0, 0));
        self.mark(phase, generation, revision, u8::from(frame.is_some()));
    }

    pub fn finish(&self, generation: u64, revision: u64) -> Option<OpeningReport> {
        #[cfg(debug_assertions)]
        if let Some(state) = &self.state {
            return lock(state).finish(generation, revision);
        }
        let _ = (generation, revision);
        None
    }
}

/// Only retired through the backend service; formatting and stderr never run on UI.
pub(super) struct OpeningReport {
    #[cfg(debug_assertions)]
    _records: opening_trace::Report,
}

#[cfg(debug_assertions)]
mod opening_trace {
    use super::*;
    const CAPACITY: usize = 64;

    #[derive(Clone, Copy, Default)]
    struct Record {
        phase: u8,
        at_us: u64,
        generation: u64,
        revision: u64,
        pending: u8,
    }

    pub(super) struct State {
        origin: Instant,
        opening: u64,
        records: [Record; CAPACITY],
        used: usize,
        lost: u64,
        pub finished: bool,
    }

    impl State {
        pub fn new(origin: Instant, opening: u64) -> Self {
            Self {
                origin,
                opening,
                records: [Record::default(); CAPACITY],
                used: 0,
                lost: 0,
                finished: false,
            }
        }

        pub fn mark(&mut self, phase: u8, generation: u64, revision: u64, pending: u8) {
            if self.finished {
                return;
            }
            // Reserve the final slot for accepted publication even after a burst of signals.
            if self.used == CAPACITY - 1 && phase != 50 {
                self.lost = self.lost.saturating_add(1);
                return;
            }
            self.records[self.used] = Record {
                phase,
                at_us: self.origin.elapsed().as_micros().min(u128::from(u64::MAX)) as u64,
                generation,
                revision,
                pending,
            };
            self.used += 1;
        }

        pub fn finish(&mut self, generation: u64, revision: u64) -> Option<OpeningReport> {
            if self.finished {
                return None;
            }
            self.mark(50, generation, revision, 0);
            self.finished = true;
            Some(OpeningReport {
                _records: Report {
                    opening: self.opening,
                    created_on: std::thread::current().id(),
                    records: self.records,
                    used: self.used,
                    lost: self.lost,
                },
            })
        }
    }

    pub(super) struct Report {
        opening: u64,
        created_on: std::thread::ThreadId,
        records: [Record; CAPACITY],
        used: usize,
        lost: u64,
    }

    impl Drop for Report {
        fn drop(&mut self) {
            // A shut-down retirement executor may discard its closure on the caller.
            // Cancellation releases the bounded report without logging on that UI thread.
            if std::thread::current().id() == self.created_on {
                return;
            }
            use std::io::Write as _;
            let stderr = std::io::stderr();
            let mut output = stderr.lock();
            for record in &self.records[..self.used] {
                let _ = writeln!(
                    output,
                    "viewer_open opening={} phase={} at_us={} generation={} revision={} pending={} lost={}",
                    self.opening,
                    record.phase,
                    record.at_us,
                    record.generation,
                    record.revision,
                    record.pending,
                    self.lost,
                );
            }
        }
    }
}

/// Retained body plus gutter bytes, from the budget the engine produces within;
/// larger frames are refused.
pub(super) const RETAINED_BYTES: u64 = midnight_core::VIEWER_RETAINED_RASTER_PIXELS as u64 * 4;
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
    pub(super) trace: OpeningTrace,
}

impl Preparation {
    pub fn new(
        mailbox: Arc<Mailbox>,
        notify: Notify,
        trace: OpeningTrace,
        palette: crate::appearance::PaletteGate,
    ) -> Result<Self, String> {
        let source = mailbox.clone();
        let mut cache = Cache::default();
        let drained = trace.clone();
        let prepared = trace.clone();
        let mut preparation = Self::start(
            move || {
                let outputs = source.take();
                drained.outputs(10, &outputs);
                outputs
            },
            move |outputs| {
                let batch = palette_batch(outputs, &mut cache, &palette);
                prepared.outputs(20, &batch.outputs);
                batch
            },
            notify,
        )?;
        preparation.trace = trace.clone();
        let shared = Arc::downgrade(&preparation.shared);
        mailbox.notify_on_ready(Arc::new(move || {
            if let Some(shared) = shared.upgrade() {
                if trace.active() {
                    let pending = {
                        let state = lock(&shared.state);
                        state.batch.as_ref().map_or(0, |batch| {
                            if batch
                                .outputs
                                .iter()
                                .any(|output| matches!(output, ViewerOutput::Frame(_)))
                            {
                                2
                            } else {
                                1
                            }
                        })
                    };
                    trace.mark(9, 0, 0, pending);
                }
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
        Ok(Self {
            shared,
            trace: OpeningTrace::default(),
        })
    }

    pub fn take(&self) -> Option<PreparedBatch> {
        let batch = lock(&self.shared.state).batch.take();
        // Only a consumed batch can release the worker's delivery wait. The UI
        // polls this every frame; an empty poll must not wake an idle worker.
        if batch.is_some() {
            self.shared.ready.notify_one();
        }
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

fn palette_batch(
    mut outputs: Vec<ViewerOutput>,
    cache: &mut Cache,
    palette: &crate::appearance::PaletteGate,
) -> PreparedBatch {
    // A reserved control may arrive in an earlier drain than its frame. Keep its
    // scalar fence independently of the one prepared batch and pixel cache.
    palette.observe(&outputs);
    outputs
        .retain(|output| !matches!(output, ViewerOutput::Frame(frame) if !palette.allows(frame)));
    let mut prepared = batch(outputs, cache);
    // Acceptance can race a large copy. Reclaim both rejected source and copied
    // buffers here on the preparation worker; the UI repeats this check at delivery.
    prepared
        .outputs
        .retain(|output| !matches!(output, ViewerOutput::Frame(frame) if !palette.allows(frame)));
    if !prepared
        .outputs
        .iter()
        .any(|output| matches!(output, ViewerOutput::Frame(_)))
    {
        prepared.pixels = Ok(None);
    }
    prepared
}

#[cfg(test)]
mod palette_tests {
    use super::*;

    #[test]
    fn preparation_saves_split_palette_fence_and_retires_pending_pixels_without_losing_controls() {
        let palette = crate::appearance::PaletteGate::default();
        let mut appearance = midnight_core::ViewerAppearance {
            family: "monospace".into(),
            font_size: 14.0,
            foreground: [255; 4],
            background: [0; 4],
            selection: [55; 4],
            accent: [150; 4],
        };
        palette.accepted(&appearance, 0);
        let mut stale = crate::text_presentation_tests::frame(11, 0, 0.2);
        if let ViewerOutput::Frame(frame) = &mut stale {
            frame.pixels = Arc::from([]); // Must be rejected before pixel validation.
        }
        let source = Arc::new(Mutex::new(std::collections::VecDeque::from([
            vec![ViewerOutput::PaletteApplied {
                revision: 0,
                generation: 7,
                minimum_frame_revision: Some(10),
            }],
            vec![crate::text_presentation_tests::frame(10, 0, 0.2)],
            vec![
                stale,
                ViewerOutput::CopyText("keep control".into()),
                ViewerOutput::Closed,
            ],
        ])));
        let drain = source.clone();
        let preparing = palette.clone();
        let mut cache = Cache::default();
        let (notify, notified) = std::sync::mpsc::channel();
        let preparation = Preparation::start(
            move || lock(&drain).pop_front().unwrap_or_default(),
            move |outputs| palette_batch(outputs, &mut cache, &preparing),
            Arc::new(move || notify.send(()).is_ok()),
        )
        .unwrap();
        notified.recv_timeout(Duration::from_secs(2)).unwrap();
        let control = preparation.take().unwrap();
        assert!(matches!(
            control.outputs.as_slice(),
            [ViewerOutput::PaletteApplied { .. }]
        ));
        assert!(matches!(control.pixels, Ok(None)));
        assert!(preparation.shared.signal());
        notified.recv_timeout(Duration::from_secs(2)).unwrap();
        let pixels = preparation.take().unwrap();
        assert!(
            matches!(pixels.outputs.as_slice(), [ViewerOutput::Frame(frame)] if frame.revision == 10)
        );
        assert_eq!(pixels.pixels.unwrap().unwrap().body.as_bytes(), &[255; 16]);
        appearance.background = [32; 4];
        palette.accepted(&appearance, 1);
        assert!(preparation.shared.signal());
        notified.recv_timeout(Duration::from_secs(2)).unwrap();
        let terminal = preparation.take().unwrap();
        assert!(
            matches!(terminal.outputs.as_slice(), [ViewerOutput::CopyText(text), ViewerOutput::Closed] if text == "keep control")
        );
        assert!(
            matches!(terminal.pixels, Ok(None)),
            "stale malformed pixels cannot become an error"
        );
        assert!(lock(&source).is_empty());
        preparation.stop();
    }
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
