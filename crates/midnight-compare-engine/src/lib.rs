//! Bounded, read-only text and image comparison. Every expensive operation runs on a worker.
#![forbid(unsafe_code)]

mod external_compare;
mod file_choice;
mod identity;
mod image_cache;
mod images;
mod policy;
mod routing;
mod text;
mod worker;

#[cfg(test)]
mod appearance_tests;
#[cfg(test)]
mod capture_tests;
#[cfg(test)]
mod external_routes_tests;
#[cfg(test)]
mod mailbox_phase_tests;
#[cfg(test)]
mod mailbox_tests;
#[cfg(test)]
mod policy_test_support;
#[cfg(test)]
mod policy_tests;
#[cfg(test)]
mod resident_test_support;
#[cfg(test)]
mod resident_tests;

pub use external_compare::ExternalCompareLauncher;
pub use policy::{FileComparatorPinError, FileComparatorPolicySource, OwnedFileComparatorPolicy};

use midnight_core::{
    Cancellation, ExternalCompareSessionCredit, FileComparatorSnapshot, FileCompareLaunch,
    PathPickerOwner, PathPickerService, ViewerAppearance, ViewerGeometry,
};
use midnight_work_budget::resident;
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    mpsc::{self, Receiver, SyncSender},
};

/// Backend-only native source capture. Failure never invokes a fallback opener.
/// The borrowed cancellation flag remains valid for the duration of this call.
pub type NativeCapture = Arc<
    dyn Fn(&std::path::Path, &AtomicBool) -> Result<midnight_plugin_host::CapturedFile, String>
        + Send
        + Sync,
>;

type ImageDiscovery = fn() -> Result<Vec<midnight_image_viewer_host::Provider>, String>;

/// Admission failed before a comparison worker or external launch was created.
#[derive(Debug)]
pub enum OpenError {
    /// Refresh both accepted policy snapshot fields once, then retry admission.
    PolicyChanged,
    SessionLimit,
    Unavailable(String),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PolicyChanged => output
                .write_str("Comparator settings changed before this comparison opened. Try again."),
            Self::SessionLimit => {
                output.write_str("Close a comparator before opening another (maximum two).")
            }
            Self::Unavailable(message) => output.write_str(message),
        }
    }
}

impl std::error::Error for OpenError {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ImageMode {
    #[default]
    SideBySide,
    Swipe,
    OnionSkin,
}

/// Whole-input identity established from captured, revision-validated sources.
/// Raster equality, viewport contents and presentation text are never evidence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IdentityVerdict {
    /// Verification is incomplete, cancelled, invalidated or failed.
    #[default]
    Pending,
    /// Every raw byte of both complete captured files was compared equal.
    IdenticalFiles,
    /// Complete decoded text is equal, while raw encoding or BOM bytes differ.
    IdenticalText,
    /// Captured raw bytes (images) or complete decoded text (text mode) differ.
    Different,
}

#[derive(Clone, Debug)]
pub enum Input {
    /// Explicit native paths, without shell expansion or implicit working directories.
    Compare([PathBuf; 2]),
    Browse {
        side: usize,
        paths: [PathBuf; 2],
        owner: PathPickerOwner,
    },
    Resize(ViewerGeometry, u64),
    Scroll {
        x: f32,
        y: f32,
    },
    Seek(f64),
    Difference {
        backwards: bool,
    },
    DifferenceEdge {
        last: bool,
    },
    FontFamily(String),
    Mode(ImageMode),
    Zoom(f64),
    ActualPixels,
    Fit,
    #[cfg(test)]
    ChooserBarrier(SyncSender<std::thread::ThreadId>),
    Close,
}

pub struct Frame {
    pub fonts: Option<FontChoices>,
    pub geometry_revision: u64,
    /// Checked monotonic identity of this exact comparison attempt, independent of geometry.
    pub comparison_generation: u64,
    pub identity: IdentityVerdict,
    pub mode: Option<ImageMode>,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub layers: Option<ImageLayers>,
    pub accessible_text: String,
    pub status: String,
    pub image: bool,
    pub position: f64,
}

pub use midnight_image_viewer_host::Frame as RasterFrame;

/// Opaque, sampled source images. Clipping and opacity are presentation only.
#[derive(Clone)]
pub struct ImageLayers {
    pub frames: [Arc<RasterFrame>; 2],
}

#[derive(Clone)]
pub struct FontChoices {
    pub families: Arc<[String]>,
    pub selected: String,
}

pub enum Output {
    ChooseFiles(FileChoice),
    Frame {
        frame: Frame,
        names: [String; 2],
        appearance_revision: u64,
    },
    Status(String),
    ExternalStarted {
        pid: u32,
        tracking_warning: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternalCompareChoiceOutcome {
    NotStarted,
    Unknown,
}

/// Editable file choice. Empty paths denote unknown inputs; retain native identities in the UI.
#[derive(Debug, Clone)]
pub struct FileChoice {
    pub paths: [PathBuf; 2],
    pub message: String,
    pub is_error: bool,
    pub external_outcome: Option<ExternalCompareChoiceOutcome>,
}

/// A latest-output slot bounds memory even when the UI is paused.
#[derive(Default)]
pub struct Mailbox(Mutex<MailboxState>);

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Palette {
    foreground: [u8; 4],
    background: [u8; 4],
    selection: [u8; 4],
    accent: [u8; 4],
}

impl From<&ViewerAppearance> for Palette {
    fn from(appearance: &ViewerAppearance) -> Self {
        Self {
            foreground: appearance.foreground,
            background: appearance.background,
            selection: appearance.selection,
            accent: appearance.accent,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct PaletteRequest {
    palette: Palette,
    revision: u64,
}

#[derive(Default)]
struct MailboxState {
    output: Option<Output>,
    // At most one superseded frame awaits reclamation by the existing worker.
    retired: Option<Output>,
    generation: u64,
    closed: bool,
    worker_connected: bool,
    phase: Phase,
    appearance: PaletteRequest,
}

impl MailboxState {
    fn matches_appearance(&self, output: &Output) -> bool {
        match output {
            Output::Frame {
                appearance_revision,
                ..
            } => *appearance_revision == self.appearance.revision,
            _ => true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Ordinary,
    ExternalPending,
    ExternalUnknown,
    ExternalStarted,
}

struct GenerationClock(AtomicU64);

impl GenerationClock {
    fn next(&self) -> Result<u64, String> {
        self.0
            .try_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1)
            })
            .map(|previous| previous + 1)
            .map_err(|_| "Comparison generation IDs exhausted.".into())
    }
}

static GENERATIONS: GenerationClock = GenerationClock(AtomicU64::new(0));

struct Command {
    input: Input,
    generation: u64,
}

impl Mailbox {
    fn new(generation: u64, palette: Palette) -> Self {
        Self(Mutex::new(MailboxState {
            generation,
            worker_connected: true,
            appearance: PaletteRequest {
                palette,
                revision: 0,
            },
            ..MailboxState::default()
        }))
    }
    pub fn take(&self) -> Option<Output> {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        // Leave an unread obsolete frame for the worker to retire.
        if state.closed
            || state
                .output
                .as_ref()
                .is_some_and(|output| !state.matches_appearance(output))
        {
            None
        } else {
            state.output.take()
        }
    }
    fn put(&self, generation: u64, output: Output) -> bool {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        let retired = state.retired.take();
        let accept = !state.closed
            && state.generation == generation
            && state.matches_appearance(&output)
            && match (state.phase, &output) {
                (Phase::ExternalStarted | Phase::ExternalUnknown, _) => false,
                (Phase::ExternalPending, Output::ExternalStarted { .. }) => {
                    state.phase = Phase::ExternalStarted;
                    true
                }
                (Phase::ExternalPending, Output::ChooseFiles(choice)) => {
                    match choice.external_outcome {
                        Some(ExternalCompareChoiceOutcome::Unknown) => {
                            state.phase = Phase::ExternalUnknown;
                            true
                        }
                        Some(ExternalCompareChoiceOutcome::NotStarted) => {
                            state.phase = Phase::Ordinary;
                            true
                        }
                        None => false,
                    }
                }
                (Phase::ExternalPending, Output::Status(_)) => true,
                (Phase::ExternalPending, _) => false,
                (Phase::Ordinary, Output::ExternalStarted { .. }) => false,
                (Phase::Ordinary, Output::ChooseFiles(choice)) => choice.external_outcome.is_none(),
                (Phase::Ordinary, _) => true,
            };
        let replaced = if accept {
            state.output.replace(output)
        } else {
            Some(output)
        };
        drop(state);
        drop((retired, replaced));
        accept
    }
    fn appearance(&self) -> PaletteRequest {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .appearance
    }
    fn begin_external(&self, generation: u64) -> bool {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if state.closed || state.generation != generation || state.phase != Phase::Ordinary {
            return false;
        }
        state.phase = Phase::ExternalPending;
        true
    }
    fn is_current(&self, generation: u64) -> bool {
        let state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        !state.closed && state.generation == generation
    }
    fn frame_waiting(&self, generation: u64) -> bool {
        let state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        state.generation == generation && matches!(state.output, Some(Output::Frame { .. }))
    }
    fn close(&self) {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        state.closed = true;
    }
    fn finish_on_worker(&self, error: Option<String>) {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        state.worker_connected = false;
        let output = if state.closed {
            state.output.take()
        } else if matches!(state.phase, Phase::ExternalStarted | Phase::ExternalUnknown) {
            None
        } else if let Some(error) = error {
            state.output.replace(Output::Status(error))
        } else if state
            .output
            .as_ref()
            .is_some_and(|output| !state.matches_appearance(output))
        {
            state.output.take()
        } else {
            None
        };
        let retired = state.retired.take();
        drop(state);
        drop((output, retired));
    }
}

#[derive(Clone)]
pub struct Service {
    handle: tokio::runtime::Handle,
    spawner: resident::Spawner,
    capture: NativeCapture,
    active: Arc<AtomicUsize>,
    picker: Option<Arc<dyn PathPickerService>>,
    policy_source: Option<Arc<dyn FileComparatorPolicySource>>,
    launcher: Option<Arc<dyn ExternalCompareLauncher>>,
    discover_images: ImageDiscovery,
}

pub struct Session {
    sender: SyncSender<Command>,
    cancelled: Arc<AtomicBool>,
    picker_cancel: Cancellation,
    pub mailbox: Arc<Mailbox>,
    handle: tokio::runtime::Handle,
    worker: resident::Ticket,
}

impl Session {
    /// Read the accepted palette revision, including after close or disconnect.
    pub fn palette_revision(&self) -> u64 {
        self.mailbox
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .appearance
            .revision
    }

    /// Store the latest render palette. The revision acknowledges acceptance,
    /// and the worker stamps completed frames with the palette it used.
    /// The worker observes this slot on its existing input-poll cadence.
    pub fn restore_palette(&self, appearance: &ViewerAppearance) -> Result<u64, String> {
        let palette = Palette::from(appearance);
        let mut state = self
            .mailbox
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.closed || !state.worker_connected {
            return Err("Comparison is closed.".into());
        }
        if state.appearance.palette == palette {
            return Ok(state.appearance.revision);
        }
        let revision = state
            .appearance
            .revision
            .checked_add(1)
            .ok_or("Comparison appearance revisions exhausted.")?;
        state.appearance = PaletteRequest { palette, revision };
        Ok(revision)
    }

    pub fn send(&self, input: Input) -> bool {
        if matches!(input, Input::Close) {
            self.close();
            return true;
        }
        let mut state = self
            .mailbox
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.closed || !state.worker_connected {
            return false;
        }
        let replacement = matches!(input, Input::Compare(_) | Input::Browse { .. });
        if replacement && matches!(state.phase, Phase::ExternalPending | Phase::ExternalStarted) {
            return false;
        }
        if replacement
            && state.retired.is_some()
            && matches!(state.output, Some(Output::Frame { .. }))
        {
            return false;
        }
        let generation = if replacement {
            let Ok(generation) = GENERATIONS.next() else {
                return false;
            };
            generation
        } else {
            state.generation
        };
        if self.sender.try_send(Command { input, generation }).is_err() {
            return false;
        }
        if replacement {
            state.generation = generation;
            state.phase = Phase::Ordinary;
            if matches!(state.output, Some(Output::Frame { .. })) {
                state.retired = state.output.take();
            }
            state.output = Some(Output::Status("Preparing file identities…".into()));
        }
        true
    }
    pub fn close(&self) {
        self.picker_cancel.cancel();
        self.cancelled.store(true, Ordering::Release);
        self.mailbox.close();
        self.worker.abort();
        let _ = self.sender.try_send(Command {
            input: Input::Close,
            generation: 0,
        });
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
impl ExternalCompareSessionCredit for Permit {}
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

// This owner marks the mailbox disconnected before dropping the only receiver,
// including when the queued job is discarded before its body runs.
struct WorkerInput {
    output: Arc<Mailbox>,
    receiver: Receiver<Command>,
    error: Option<String>,
}

impl Drop for WorkerInput {
    fn drop(&mut self) {
        self.output.finish_on_worker(self.error.take());
    }
}

struct ComparisonWork {
    input: WorkerInput,
    stop: Arc<AtomicBool>,
    capture: NativeCapture,
    launch: FileCompareLaunch,
    appearance: ViewerAppearance,
    geometry: ViewerGeometry,
    generation: u64,
    policy: Option<Box<dyn OwnedFileComparatorPolicy>>,
    launcher: Option<Arc<dyn ExternalCompareLauncher>>,
    credit: Arc<dyn ExternalCompareSessionCredit>,
    cancellation: Cancellation,
    discover_images: ImageDiscovery,
    picker: file_choice::Picker,
    #[cfg(test)]
    queued: std::time::Instant,
    // Keep this last: skipped and rejected envelopes drop their captured
    // work before releasing admission. Policy/process retirement may hold
    // the same credit beyond the envelope's lifetime.
    _permit: Arc<Permit>,
}

impl ComparisonWork {
    fn run(mut self) {
        let output = &self.input.output;
        #[cfg(test)]
        eprintln!(
            "Comparator worker={:?} mailbox={:p} phase=worker-started queued_for={:?}",
            std::thread::current().id(),
            output.as_ref(),
            self.queued.elapsed()
        );
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            worker::run(
                self.launch,
                self.appearance,
                self.geometry,
                self.generation,
                worker::Context {
                    receiver: &self.input.receiver,
                    output,
                    capture: worker::Capture {
                        factory: &self.capture,
                        cancel: &self.stop,
                    },
                    policy: self.policy,
                    launcher: self.launcher,
                    credit: self.credit,
                    cancellation: self.cancellation,
                    discover_images: self.discover_images,
                },
                self.picker,
            )
        }))
        .unwrap_or_else(|_| Err("Comparison stopped after an internal error.".into()));
        self.input.error = result.err().filter(|_| !self.stop.load(Ordering::Acquire));
    }
}

// If Tokio can no longer run retirement, retain the refused work and its
// admission credit. At most two such envelopes can exist for this service.
struct RejectedComparison(Option<resident::Work>);

impl RejectedComparison {
    fn retire(mut self) {
        drop(self.0.take());
    }
}

impl Drop for RejectedComparison {
    fn drop(&mut self) {
        if let Some(work) = self.0.take() {
            std::mem::forget(work);
        }
    }
}

impl Service {
    pub fn new(handle: tokio::runtime::Handle, spawner: resident::Spawner) -> Self {
        Self::with_native_capture(
            handle,
            spawner,
            Arc::new(|path, cancel| {
                check_cancel(cancel)?;
                midnight_plugin_host::CapturedFile::capture(path).map_err(|error| error.to_string())
            }),
        )
    }
    /// Install the filesystem's capture authority for both comparison sources.
    /// Invoked only by comparison workers. The callback must retain its provider
    /// for the lifetime of every returned content lease; errors never fall back.
    pub fn with_native_capture(
        handle: tokio::runtime::Handle,
        spawner: resident::Spawner,
        capture: NativeCapture,
    ) -> Self {
        Self {
            handle,
            spawner,
            capture,
            active: Arc::new(AtomicUsize::new(0)),
            picker: None,
            policy_source: None,
            launcher: None,
            discover_images: routing::discover,
        }
    }
    pub fn with_path_picker(mut self, picker: Arc<dyn PathPickerService>) -> Self {
        self.picker = Some(picker);
        self
    }
    pub fn has_path_picker(&self) -> bool {
        self.picker.is_some()
    }
    /// Install the backend's guarded accepted-policy source, including final retirement.
    pub fn with_comparator_policies(mut self, source: Arc<dyn FileComparatorPolicySource>) -> Self {
        self.policy_source = Some(source);
        self
    }
    /// Install the synchronous backend command adapter for configured external programs.
    pub fn with_external_launcher(mut self, launcher: Arc<dyn ExternalCompareLauncher>) -> Self {
        self.launcher = Some(launcher);
        self
    }
    /// Read accepted raw metadata for the caller's one typed stale-policy retry.
    pub fn comparator_snapshot(&self) -> FileComparatorSnapshot {
        self.policy_source
            .as_ref()
            .map_or_else(FileComparatorSnapshot::default, |source| source.snapshot())
    }
    pub fn open(
        &self,
        launch: FileCompareLaunch,
        appearance: ViewerAppearance,
        geometry: ViewerGeometry,
    ) -> Result<Session, OpenError> {
        self.active
            .try_update(Ordering::AcqRel, Ordering::Relaxed, |count| {
                (count < 2).then_some(count + 1)
            })
            .map_err(|_| OpenError::SessionLimit)?;
        let permit = Arc::new(Permit(self.active.clone()));
        let credit: Arc<dyn ExternalCompareSessionCredit> = permit.clone();
        let generation = GENERATIONS.next().map_err(OpenError::Unavailable)?;
        let (sender, receiver) = mpsc::sync_channel(32);
        let mailbox = Arc::new(Mailbox::new(generation, Palette::from(&appearance)));
        let cancelled = Arc::new(AtomicBool::new(false));
        let input = WorkerInput {
            output: mailbox.clone(),
            receiver,
            error: None,
        };
        let stop = cancelled.clone();
        let capture = self.capture.clone();
        let picker_cancel = Cancellation::default();
        let picker = file_choice::Picker {
            service: self.picker.clone(),
            cancel: picker_cancel.clone(),
        };
        let launcher = self.launcher.clone();
        let discover_images = self.discover_images;
        let cancellation = Cancellation::observing(stop.clone());
        let expected = FileComparatorSnapshot {
            id: launch.comparator_policy,
            preferences: launch.comparators.clone(),
        };
        // Complete fallible local setup before accepting a historical policy.
        // The backend's already armed owner covers every later closure-drop path.
        let policy = if let Some(source) = &self.policy_source {
            Some(
                source
                    .pin(&expected, credit.clone())
                    .map_err(|error| match error {
                        FileComparatorPinError::PolicyChanged => OpenError::PolicyChanged,
                        FileComparatorPinError::RetirementBusy => {
                            OpenError::Unavailable(error.to_string())
                        }
                    })?,
            )
        } else {
            if !policy::is_policy_free_default(&expected) {
                return Err(OpenError::Unavailable(
                    "Configured comparator settings are unavailable for this service.".into(),
                ));
            }
            None
        };
        let work = ComparisonWork {
            input,
            stop,
            capture,
            launch,
            appearance,
            geometry,
            generation,
            policy,
            launcher,
            credit,
            cancellation,
            discover_images,
            picker,
            #[cfg(test)]
            queued: std::time::Instant::now(),
            _permit: permit,
        };
        let worker = match self
            .spawner
            .spawn(&self.handle, Box::new(move || work.run()))
        {
            Ok(worker) => worker,
            Err(refused) => {
                let message = match refused.reason() {
                    resident::Rejection::Full => "Comparison workers are busy. Try again.",
                    resident::Rejection::Sealed => "Comparison workers are shutting down.",
                    resident::Rejection::WorkerFailed => "Comparison workers are unavailable.",
                };
                let rejected = RejectedComparison(Some(refused.into_work()));
                self.handle.spawn_blocking(move || rejected.retire());
                return Err(OpenError::Unavailable(message.into()));
            }
        };
        Ok(Session {
            sender,
            cancelled,
            picker_cancel,
            mailbox,
            handle: self.handle.clone(),
            worker,
        })
    }
    pub fn retire<T: Send + 'static>(&self, value: T) {
        self.handle.spawn_blocking(move || drop(value));
    }
}

#[cfg(test)]
fn trace_first_frame(phase: &str, started: std::time::Instant) {
    eprintln!(
        "Comparator worker={:?} phase={phase} elapsed={:?}",
        std::thread::current().id(),
        started.elapsed()
    );
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Comparison cancelled.".into())
    } else {
        Ok(())
    }
}
