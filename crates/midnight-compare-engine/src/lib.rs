//! Bounded, read-only text, image and byte comparison. Every expensive operation runs on a
//! worker.
#![forbid(unsafe_code)]

mod binary;
mod discovery;
mod external_compare;
mod file_choice;
mod identity;
mod image_cache;
mod images;
mod policy;
mod revalidation;
mod routing;
mod text;
mod typeface;
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
    Arc, Mutex, MutexGuard,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    mpsc::{self, Receiver, SyncSender, TrySendError},
};

/// Backend-only native source capture. Failure never invokes a fallback opener.
/// The borrowed cancellation flag remains valid for the duration of this call.
pub type NativeCapture = Arc<
    dyn Fn(&std::path::Path, &AtomicBool) -> Result<midnight_plugin_host::CapturedFile, String>
        + Send
        + Sync,
>;

/// Backend-only storage query: true only when the native path is positively identified
/// as local solid-state storage. Its sources then share the process-wide I/O gate.
pub type StorageHint = Arc<dyn Fn(&std::path::Path) -> bool + Send + Sync>;

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
    /// Logical pixels, e.g. a wheel; text converts them with its painted row height.
    Scroll {
        x: f32,
        y: f32,
    },
    /// Keyboard line steps: exactly one text row each, or a fixed image pan.
    Lines(i32),
    /// Keyboard page steps: the fully visible text rows, or one viewport of an image.
    Pages(i32),
    Seek(f64),
    Difference {
        backwards: bool,
    },
    DifferenceEdge {
        last: bool,
    },
    /// Literal search over both decoded text panes, executed incrementally on the worker.
    Find {
        query: FindQuery,
        backwards: bool,
        revision: u64,
    },
    /// Relative match navigation for the accepted query, accumulated while it is scanning.
    FindStep {
        delta: i64,
        revision: u64,
    },
    FontFamily(String),
    /// Show the byte comparison (hex view) of a text pair, or its text again. Pairs
    /// that are not text are always shown as bytes and ignore it.
    Bytes(bool),
    Mode(ImageMode),
    Zoom(f64),
    ActualPixels,
    Fit,
    #[cfg(test)]
    ChooserBarrier(SyncSender<std::thread::ThreadId>),
    Close,
}

pub struct Frame {
    /// Immutable worker snapshot shared by paints until Find state changes.
    pub find: Arc<FindState>,
    pub fonts: Option<FontChoices>,
    /// Tests only: the family a text frame was painted in.
    #[cfg(test)]
    pub(crate) text_family: Option<String>,
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
    /// The frame shows the byte comparison (hex view).
    pub bytes: bool,
    /// The pair can only be compared as bytes (not text, or not two images).
    pub bytes_only: bool,
    /// A byte comparison's scan still runs; taking this frame lets the worker go on.
    pub scanning: bool,
    pub position: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FindQuery {
    pub text: String,
    pub case_sensitive: bool,
    pub whole_words: bool,
}

/// Search presentation belongs to the same comparison generation as its containing frame.
#[derive(Clone, Debug, Default)]
pub struct FindState {
    pub revision: u64,
    pub searching: bool,
    pub matches: usize,
    /// One-based ordinal across both panes. Zero denotes no selected match.
    pub active: usize,
    pub side: usize,
    pub line: usize,
    pub message: String,
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

/// Maximum number of commands and wake messages queued for a session's worker.
pub const INPUT_QUEUE: usize = 32;

/// The input queue carries commands and signals to recheck shared state.
enum Message {
    Input(Command),
    Wake,
}

/// A font catalog, or `None` when the scan was cancelled, failed or panicked.
pub(crate) type FontReply = Option<Arc<[String]>>;

/// The comparison worker's queue, and what the worker reads before every blocking receive:
/// whether the session was closed, and the reply to the font catalog request its text
/// comparison waits for. Writers change that state under the lock and then try to queue a
/// message, which a full queue may refuse without losing anything. If the worker's last
/// read came after the change, it saw the change. Otherwise every message the worker took
/// before that read was taken before the refused send, so the full queue held messages it
/// had not taken: its next receive returns at once, and it reads again after that batch.
/// Neither a close nor a reply therefore waits for room, and a window with a full queue
/// cannot delay the reply to another.
pub(crate) struct Inbox {
    queue: SyncSender<Message>,
    state: Mutex<Signals>,
}

#[derive(Default)]
struct Signals {
    closed: bool,
    /// The number of the session's latest font catalog request.
    requests: u64,
    /// A font catalog reply and the number of the request it answers.
    fonts: Option<(u64, FontReply)>,
}

impl Inbox {
    fn new(queue: SyncSender<Message>) -> Self {
        Self {
            queue,
            state: Mutex::default(),
        }
    }

    fn state(&self) -> MutexGuard<'_, Signals> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn send(&self, command: Command) -> Result<(), Refused> {
        self.queue
            .try_send(Message::Input(command))
            .map_err(|error| match error {
                TrySendError::Full(_) => Refused::Full,
                TrySendError::Disconnected(_) => Refused::Stopped,
            })
    }

    /// Ends the worker before its next blocking receive, also when every queue slot is
    /// taken; the queued wake only ends a waiting worker sooner.
    fn close(&self) {
        self.state().closed = true;
        self.wake();
    }

    /// Shared state is written before this wake. A full input queue already wakes the worker.
    fn wake(&self) {
        let _ = self.queue.try_send(Message::Wake);
    }

    /// Numbers a font catalog request of the worker's current comparison.
    pub(crate) fn font_request(&self) -> u64 {
        let mut state = self.state();
        state.requests += 1;
        state.requests
    }

    /// The scan's reply to request `request`. It runs on the scan thread and never waits
    /// for room in the queue.
    pub(crate) fn answer(&self, request: u64, reply: FontReply) {
        {
            let mut state = self.state();
            // A late reply to an earlier request must not replace the newer one.
            if state
                .fonts
                .as_ref()
                .is_none_or(|(answered, _)| *answered <= request)
            {
                state.fonts = Some((request, reply));
            }
        }
        self.wake();
    }

    /// Read before every blocking receive: `Err` once the session was closed, otherwise
    /// the reply to request `awaiting` if it arrived. A reply to an earlier request, from
    /// a comparison that failed before its reply, is dropped.
    pub(crate) fn check(&self, awaiting: Option<u64>) -> Result<Option<FontReply>, String> {
        let mut state = self.state();
        if state.closed {
            return Err("Comparison cancelled.".into());
        }
        Ok(match state.fonts.take() {
            Some((request, reply)) if Some(request) == awaiting => Some(reply),
            _ => None,
        })
    }

    /// Tests only: the request whose reply waits to be read.
    #[cfg(test)]
    pub(crate) fn answered(&self) -> Option<u64> {
        self.state().fonts.as_ref().map(|(request, _)| *request)
    }
}

/// Why `Session::send` refused an input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// The session cannot queue more input until the worker catches up.
    Full,
    /// The session has closed or the worker has stopped taking input.
    Stopped,
    /// The requested replacement is no longer allowed for this comparison.
    Rejected,
}

/// Called on the worker thread when an output is ready and no earlier call is outstanding.
/// Returns whether the consumer will be woken; when it will not, the next output calls
/// again. It must not block: the UI uses it to schedule one event-loop callback.
pub type Wake = Arc<dyn Fn() -> bool + Send + Sync>;

/// A latest-output slot bounds memory even when the UI is paused. A burst of outputs
/// schedules at most one outstanding consumer wake, rearmed by the consumer's next take.
#[derive(Default)]
pub struct Mailbox {
    state: Mutex<MailboxState>,
    wake: Option<Wake>,
    woken: AtomicBool,
    inbox: Option<Arc<Inbox>>,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Palette {
    foreground: [u8; 4],
    background: [u8; 4],
    selection: [u8; 4],
    selection_text: [u8; 4],
    accent: [u8; 4],
}

impl From<&ViewerAppearance> for Palette {
    fn from(appearance: &ViewerAppearance) -> Self {
        Self {
            foreground: appearance.foreground,
            background: appearance.background,
            selection: appearance.selection,
            selection_text: appearance.selection_text,
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
        Self {
            state: Mutex::new(MailboxState {
                generation,
                worker_connected: true,
                appearance: PaletteRequest {
                    palette,
                    revision: 0,
                },
                ..MailboxState::default()
            }),
            ..Self::default()
        }
    }

    /// Clear the outstanding wake before reading the slot so a concurrent publication
    /// either reaches this take or schedules another wake.
    pub fn take(&self) -> Option<Output> {
        self.woken.swap(false, Ordering::AcqRel);
        let output = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            // Leave an unread obsolete frame for the worker to retire.
            if state.closed
                || state
                    .output
                    .as_ref()
                    .is_some_and(|output| !state.matches_appearance(output))
            {
                return None;
            }
            state.output.take()
        };
        if let Some(Output::Frame { frame, .. }) = &output
            && (frame.identity == IdentityVerdict::Pending || frame.scanning)
            && let Some(inbox) = &self.inbox
        {
            // Whole-input verification and the byte scan run only after the consumer
            // takes the frame.
            inbox.wake();
        }
        output
    }

    fn wake_consumer(&self) {
        if let Some(wake) = &self.wake
            && !self.woken.swap(true, Ordering::AcqRel)
            && !wake()
        {
            // Let the next output try again if the consumer could not be woken.
            self.woken.swap(false, Ordering::AcqRel);
        }
    }
    fn put(&self, generation: u64, output: Output) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
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
        if accept {
            self.wake_consumer();
        }
        accept
    }
    fn appearance(&self) -> PaletteRequest {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .appearance
    }
    fn begin_external(&self, generation: u64) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.closed || state.generation != generation || state.phase != Phase::Ordinary {
            return false;
        }
        state.phase = Phase::ExternalPending;
        true
    }
    fn is_current(&self, generation: u64) -> bool {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        !state.closed && state.generation == generation
    }
    fn frame_waiting(&self, generation: u64) -> bool {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.generation == generation && matches!(state.output, Some(Output::Frame { .. }))
    }
    fn close(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.closed = true;
    }
    fn finish_on_worker(&self, error: Option<String>) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.worker_connected = false;
        let mut published_error = false;
        let output = if state.closed {
            state.output.take()
        } else if matches!(state.phase, Phase::ExternalStarted | Phase::ExternalUnknown) {
            None
        } else if let Some(error) = error {
            published_error = true;
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
        if published_error {
            self.wake_consumer();
        }
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
    storage: Option<StorageHint>,
    viewers: Arc<discovery::ImageViewers>,
    fonts: Arc<discovery::FixedPitchFonts>,
    #[cfg(test)]
    turns: Option<Arc<AtomicUsize>>,
}

pub struct Session {
    inbox: Arc<Inbox>,
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
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .appearance
            .revision
    }

    /// Store the latest render palette. The revision acknowledges acceptance,
    /// and the worker stamps completed frames with the palette it used.
    /// The worker wakes to observe a changed palette even if it has no input queued.
    pub fn restore_palette(&self, appearance: &ViewerAppearance) -> Result<u64, String> {
        let palette = Palette::from(appearance);
        let mut state = self
            .mailbox
            .state
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
        drop(state);
        self.inbox.wake();
        Ok(revision)
    }

    /// Queues an input without blocking and reports why it could not be accepted.
    pub fn send(&self, input: Input) -> Result<(), Refused> {
        if matches!(input, Input::Close) {
            self.close();
            return Ok(());
        }
        let mut state = self
            .mailbox
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.closed || !state.worker_connected {
            return Err(Refused::Stopped);
        }
        let replacement = matches!(input, Input::Compare(_) | Input::Browse { .. });
        if replacement && matches!(state.phase, Phase::ExternalPending | Phase::ExternalStarted) {
            return Err(Refused::Rejected);
        }
        if replacement
            && state.retired.is_some()
            && matches!(state.output, Some(Output::Frame { .. }))
        {
            return Err(Refused::Full);
        }
        let generation = if replacement {
            let Ok(generation) = GENERATIONS.next() else {
                return Err(Refused::Rejected);
            };
            generation
        } else {
            state.generation
        };
        self.inbox.send(Command { input, generation })?;
        if replacement {
            state.generation = generation;
            state.phase = Phase::Ordinary;
            if matches!(state.output, Some(Output::Frame { .. })) {
                state.retired = state.output.take();
            }
            state.output = Some(Output::Status("Preparing file identities…".into()));
        }
        Ok(())
    }
    pub fn close(&self) {
        self.picker_cancel.cancel();
        self.cancelled.store(true, Ordering::Release);
        self.mailbox.close();
        self.worker.abort();
        self.inbox.close();
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
    receiver: Receiver<Message>,
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
    inbox: Arc<Inbox>,
    storage: Option<StorageHint>,
    viewers: Arc<discovery::ImageViewers>,
    fonts: Arc<discovery::FixedPitchFonts>,
    picker: file_choice::Picker,
    #[cfg(test)]
    turns: Option<Arc<AtomicUsize>>,
    // Keep this last: skipped and rejected envelopes drop their captured
    // work before releasing admission. Policy/process retirement may hold
    // the same credit beyond the envelope's lifetime.
    _permit: Arc<Permit>,
}

impl ComparisonWork {
    fn run(mut self) {
        let output = &self.input.output;
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
                        storage: self.storage.as_ref(),
                        viewers: &self.viewers,
                        fonts: &self.fonts,
                        inbox: &self.inbox,
                        cancel: &self.stop,
                        #[cfg(test)]
                        turns: self.turns.as_deref(),
                    },
                    policy: self.policy,
                    launcher: self.launcher,
                    credit: self.credit,
                    cancellation: self.cancellation,
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
            storage: None,
            viewers: Arc::new(discovery::ImageViewers::installed()),
            fonts: Arc::new(discovery::FixedPitchFonts::installed()),
            #[cfg(test)]
            turns: None,
        }
    }
    #[cfg(test)]
    fn with_image_viewer_discovery(
        mut self,
        discover: impl Fn() -> Result<Vec<midnight_image_viewer_host::Provider>, String>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.viewers = Arc::new(discovery::ImageViewers::with_discovery(discover));
        self
    }
    #[cfg(test)]
    fn with_font_scan(
        mut self,
        scan: impl Fn(&dyn Fn() -> bool) -> Option<Arc<[String]>> + Send + Sync + 'static,
    ) -> Self {
        self.fonts = Arc::new(discovery::FixedPitchFonts::with_scan(scan));
        self
    }
    /// Replaces the check of the preferred family that runs before a catalog is kept.
    #[cfg(test)]
    fn with_font_check(
        mut self,
        check: impl Fn(&[&str], &dyn Fn() -> bool) -> Option<String> + Send + Sync + 'static,
    ) -> Self {
        self.fonts_mut().set_check(check);
        self
    }
    /// The font scan cannot get a thread, as when the system refuses one.
    #[cfg(test)]
    fn without_font_thread(mut self) -> Self {
        self.fonts_mut().unspawnable = true;
        self
    }
    #[cfg(test)]
    fn fonts_mut(&mut self) -> &mut discovery::FixedPitchFonts {
        Arc::get_mut(&mut self.fonts).expect("configure fonts before opening a session")
    }
    #[cfg(test)]
    fn with_turn_counter(mut self, turns: Arc<AtomicUsize>) -> Self {
        self.turns = Some(turns);
        self
    }
    pub fn with_path_picker(mut self, picker: Arc<dyn PathPickerService>) -> Self {
        self.picker = Some(picker);
        self
    }
    /// Without a hint every comparison read is admitted exclusively, like unknown storage.
    pub fn with_storage_hint(mut self, storage: StorageHint) -> Self {
        self.storage = Some(storage);
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
    /// Open a comparison for a caller that reads the session mailbox directly.
    pub fn open(
        &self,
        launch: FileCompareLaunch,
        appearance: ViewerAppearance,
        geometry: ViewerGeometry,
    ) -> Result<Session, OpenError> {
        self.start(launch, appearance, geometry, None)
    }

    /// Open a comparison whose worker wakes the consumer when a result is ready.
    pub fn open_with_wake(
        &self,
        launch: FileCompareLaunch,
        appearance: ViewerAppearance,
        geometry: ViewerGeometry,
        wake: Wake,
    ) -> Result<Session, OpenError> {
        self.start(launch, appearance, geometry, Some(wake))
    }

    fn start(
        &self,
        launch: FileCompareLaunch,
        appearance: ViewerAppearance,
        geometry: ViewerGeometry,
        wake: Option<Wake>,
    ) -> Result<Session, OpenError> {
        self.active
            .try_update(Ordering::AcqRel, Ordering::Relaxed, |count| {
                (count < 2).then_some(count + 1)
            })
            .map_err(|_| OpenError::SessionLimit)?;
        let permit = Arc::new(Permit(self.active.clone()));
        let credit: Arc<dyn ExternalCompareSessionCredit> = permit.clone();
        let generation = GENERATIONS.next().map_err(OpenError::Unavailable)?;
        let (sender, receiver) = mpsc::sync_channel(INPUT_QUEUE);
        let inbox = Arc::new(Inbox::new(sender));
        let mailbox = Arc::new(Mailbox {
            wake,
            inbox: Some(inbox.clone()),
            ..Mailbox::new(generation, Palette::from(&appearance))
        });
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
            inbox: inbox.clone(),
            storage: self.storage.clone(),
            viewers: self.viewers.clone(),
            fonts: self.fonts.clone(),
            picker,
            #[cfg(test)]
            turns: self.turns.clone(),
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
            inbox,
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

fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Comparison cancelled.".into())
    } else {
        Ok(())
    }
}
