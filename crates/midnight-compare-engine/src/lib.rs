//! Bounded, read-only text and image comparison. Every expensive operation runs on a worker.
#![forbid(unsafe_code)]

mod discovery;
mod file_choice;
mod image_cache;
mod images;
mod text;
mod worker;

#[cfg(test)]
mod capture_tests;
#[cfg(test)]
mod mailbox_tests;

use midnight_core::{
    Cancellation, FileCompareLaunch, PathPickerOwner, PathPickerService, ViewerAppearance,
    ViewerGeometry,
};
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex, MutexGuard,
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc::{self, SyncSender, TrySendError},
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ImageMode {
    #[default]
    SideBySide,
    Swipe,
    OnionSkin,
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
    FontFamily(String),
    Mode(ImageMode),
    Zoom(f64),
    ActualPixels,
    Fit,
    Close,
}

pub struct Frame {
    pub fonts: Option<FontChoices>,
    /// Tests only: the family a text frame was painted in.
    #[cfg(test)]
    pub(crate) text_family: Option<String>,
    pub geometry_revision: u64,
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
    Frame { frame: Frame, names: [String; 2] },
    Status(String),
}

/// Editable file choice. Empty paths denote unknown inputs; retain native identities in the UI.
#[derive(Debug, Clone)]
pub struct FileChoice {
    pub paths: [PathBuf; 2],
    pub message: String,
    pub is_error: bool,
}

/// Inputs a session queues for its worker; `Session::send` refuses more (`Refused::Full`)
/// until the worker takes some.
pub const INPUT_QUEUE: usize = 32;

/// A message for the comparison worker: an input, or a wake for a font catalog reply that
/// the scan left in the session's `Inbox`. The worker always blocks in `recv` instead of
/// polling for the scan.
pub(crate) enum Message {
    Input(Input),
    Fonts,
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

    fn send(&self, input: Input) -> Result<(), Refused> {
        self.queue
            .try_send(Message::Input(input))
            .map_err(|error| match error {
                TrySendError::Full(_) => Refused::Full,
                TrySendError::Disconnected(_) => Refused::Stopped,
            })
    }

    /// Ends the worker before its next blocking receive, also when every queue slot is
    /// taken; the queued Close only ends a waiting worker sooner.
    fn close(&self) {
        self.state().closed = true;
        let _ = self.queue.try_send(Message::Input(Input::Close));
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
        let _ = self.queue.try_send(Message::Fonts);
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
    /// `INPUT_QUEUE` inputs are waiting; the worker takes more as it catches up.
    Full,
    /// The worker has stopped, after `Session::close` or an internal error, and takes no
    /// more input.
    Stopped,
}

/// Called on the worker thread when an output is ready and no earlier call is outstanding.
/// Returns whether the consumer will be woken; when it will not, the next output calls
/// again. It must not block: the UI uses it to schedule one event-loop callback.
pub type Wake = Arc<dyn Fn() -> bool + Send + Sync>;

/// A latest-output slot bounds memory even when the UI is paused. With a wake, a burst of
/// outputs calls it once: the call stays outstanding until the consumer's next `take`.
#[derive(Default)]
pub struct Mailbox {
    output: Mutex<Option<Output>>,
    wake: Option<Wake>,
    /// Whether a wake is outstanding. `take` clears it before it locks the slot and `put`
    /// sets it after it unlocks the slot, so the slot's lock orders the two: either the
    /// take sees the output, or its clear came first and the put wakes again.
    woken: AtomicBool,
}
impl Mailbox {
    fn with_wake(wake: Option<Wake>) -> Self {
        Self {
            wake,
            ..Self::default()
        }
    }
    /// Takes the latest output. It first ends the outstanding wake, so an output stored
    /// after this point wakes the consumer again.
    pub fn take(&self) -> Option<Output> {
        self.woken.swap(false, Ordering::AcqRel);
        self.output
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
    }
    fn put(&self, output: Output) {
        *self
            .output
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(output);
        if let Some(wake) = &self.wake
            && !self.woken.swap(true, Ordering::AcqRel)
            && !wake()
        {
            // Nobody will take this output now: let the next one try again.
            self.woken.swap(false, Ordering::AcqRel);
        }
    }
}

#[derive(Clone)]
pub struct Service {
    handle: tokio::runtime::Handle,
    capture: NativeCapture,
    storage: Option<StorageHint>,
    viewers: Arc<discovery::ImageViewers>,
    fonts: Arc<discovery::FixedPitchFonts>,
    active: Arc<AtomicUsize>,
    picker: Option<Arc<dyn PathPickerService>>,
    /// Tests only: counts the turns of comparison workers' input loops.
    #[cfg(test)]
    turns: Option<Arc<AtomicUsize>>,
}

pub struct Session {
    inbox: Arc<Inbox>,
    cancelled: Arc<AtomicBool>,
    picker_cancel: Cancellation,
    pub mailbox: Arc<Mailbox>,
    handle: tokio::runtime::Handle,
}

impl Session {
    /// Queues an input for the worker without blocking.
    pub fn send(&self, input: Input) -> Result<(), Refused> {
        self.inbox.send(input)
    }
    /// Running reads and decodes stop at their next cancellation check; a worker that
    /// waits for input stops through its inbox, which needs no free queue slot.
    pub fn close(&self) {
        self.picker_cancel.cancel();
        self.cancelled.store(true, Ordering::Release);
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
            storage: None,
            viewers: Arc::new(discovery::ImageViewers::installed()),
            fonts: Arc::new(discovery::FixedPitchFonts::installed()),
            active: Arc::new(AtomicUsize::new(0)),
            picker: None,
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
    /// A session whose consumer polls `Session::mailbox`; tests and headless tools use it,
    /// windows use `open_with_wake`.
    pub fn open(
        &self,
        launch: FileCompareLaunch,
        appearance: ViewerAppearance,
        geometry: ViewerGeometry,
    ) -> Result<Session, String> {
        self.start(launch, appearance, geometry, None)
    }
    /// A session that calls `wake` on its worker when an output is ready (see `Mailbox`),
    /// so the consumer sleeps until then instead of polling the mailbox.
    pub fn open_with_wake(
        &self,
        launch: FileCompareLaunch,
        appearance: ViewerAppearance,
        geometry: ViewerGeometry,
        wake: Wake,
    ) -> Result<Session, String> {
        self.start(launch, appearance, geometry, Some(wake))
    }
    fn start(
        &self,
        launch: FileCompareLaunch,
        appearance: ViewerAppearance,
        geometry: ViewerGeometry,
        wake: Option<Wake>,
    ) -> Result<Session, String> {
        self.active
            .try_update(Ordering::AcqRel, Ordering::Relaxed, |count| {
                (count < 2).then_some(count + 1)
            })
            .map_err(|_| "Close a comparator before opening another (maximum two).")?;
        let permit = Permit(self.active.clone());
        let (sender, receiver) = mpsc::sync_channel(INPUT_QUEUE);
        // The worker shares the inbox with the session and its font catalog requests, so
        // its queue never disconnects while it runs: the inbox's `closed` ends it.
        let inbox = Arc::new(Inbox::new(sender));
        let worker_inbox = inbox.clone();
        #[cfg(test)]
        let turns = self.turns.clone();
        let mailbox = Arc::new(Mailbox::with_wake(wake));
        let cancelled = Arc::new(AtomicBool::new(false));
        let output = mailbox.clone();
        let stop = cancelled.clone();
        let capture = self.capture.clone();
        let storage = self.storage.clone();
        let viewers = self.viewers.clone();
        let fonts = self.fonts.clone();
        let picker_cancel = Cancellation::default();
        let picker = file_choice::Picker {
            service: self.picker.clone(),
            cancel: picker_cancel.clone(),
        };
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
                        storage: storage.as_ref(),
                        viewers: &viewers,
                        fonts: &fonts,
                        inbox: &worker_inbox,
                        cancel: &stop,
                        #[cfg(test)]
                        turns: turns.as_deref(),
                    },
                    picker,
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
            inbox,
            cancelled,
            picker_cancel,
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
