//! Backend-owned drag/drop paths and admission. Native incoming drops are Copy-only;
//! Windows outgoing supports receiver-owned Copy/Move, while Linux outgoing is Copy-only.
use crate::{
    Cancellation, FileClipboard, FileClipboardIntent, FsError, NavigationId, PaneId,
    TransferIntent, TransferVerification,
};
use std::{
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DragId(pub u64);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DragPreviewRevision(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DropToken(pub u64);

/// An internal drop identifies an acknowledged model occurrence, never a display path.
#[derive(Clone, Debug)]
pub enum InternalDropDestination {
    PaneRoot {
        pane: PaneId,
        navigation: NavigationId,
    },
    PanelEntry {
        pane: PaneId,
        navigation: NavigationId,
        entry: crate::EntryId,
    },
    TreeOccurrence {
        view: Arc<crate::TreeProjection>,
        row: crate::TreeRowId,
        request: crate::TreeRequestId,
    },
}

#[derive(Clone, Debug)]
pub struct InternalDropRequest {
    pub id: DragId,
    pub destination: InternalDropDestination,
    pub intent: InternalDropIntent,
    pub verification: crate::TransferVerification,
}

/// Latest speculative hover only. This never reserves transfer admission or
/// grants release authority; the actual drop resolves its destination again.
#[derive(Clone, Debug)]
pub struct InternalDropPreviewRequest {
    pub id: DragId,
    pub revision: DragPreviewRevision,
    pub destination: InternalDropDestination,
    pub intent: InternalDropIntent,
    /// Cancel synchronously when the pointer, modifiers or gesture change,
    /// even if a newer command cannot yet enter the bounded backend queue.
    pub cancel: Cancellation,
}

/// Modifier choice for an internal drag. Auto moves only when every source and
/// destination has the same known native device; unknown or mixed devices copy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InternalDropIntent {
    #[default]
    Auto,
    Copy,
    Move,
}

/// Provider-retained authority for one ordinary native folder. Resolve and release
/// final ownership only on backend workers. Navigation revokes an unfinished capture.
pub trait CapturedDropDirectory: std::fmt::Debug + Send + Sync {
    /// Consume on a backend worker, acknowledging actual acquired last-owner
    /// cleanup. Shared references release independently of other tabs. Unknown
    /// owners remain retained; successful wrapper Drop is not an acknowledgement.
    fn release_on_worker(self: Arc<Self>) -> Result<(), FsError> {
        std::mem::forget(self);
        Err(crate::transfer::unconfirmed_transfer_release(
            "release captured drop directory",
        ))
    }
    fn stamp(&self) -> crate::TreeViewStamp;
    fn row(&self) -> crate::TreeRowId;
    fn resolve(&self, cancel: &Cancellation) -> Result<PathBuf, FsError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeDragWindow {
    Windows {
        hwnd: usize,
    },
    X11 {
        window: u32,
    },
    /// Native Wayland lacks the same-connection input serial required for external DND.
    Unsupported,
}

#[derive(Clone, Debug)]
pub struct DragHost {
    pub generation: u64,
    pub native: NativeDragWindow,
    /// UI cancels synchronously before destroying the native window, even under backpressure.
    pub lifetime: Cancellation,
}

/// Physical client pixels. Right and bottom edges are excluded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DropRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
impl DropRect {
    pub fn contains(self, x: i32, y: i32) -> bool {
        i64::from(x) >= i64::from(self.x)
            && i64::from(x) < i64::from(self.x) + i64::from(self.width)
            && i64::from(y) >= i64::from(self.y)
            && i64::from(y) < i64::from(self.y) + i64::from(self.height)
    }
    fn overlaps(self, other: Self) -> bool {
        i64::from(self.x) < i64::from(other.x) + i64::from(other.width)
            && i64::from(other.x) < i64::from(self.x) + i64::from(self.width)
            && i64::from(self.y) < i64::from(other.y) + i64::from(other.height)
            && i64::from(other.y) < i64::from(self.y) + i64::from(self.height)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDropTarget {
    pub pane: PaneId,
    pub navigation: NavigationId,
    pub directory: Arc<PathBuf>,
    pub rect: DropRect,
}

#[derive(Clone, Debug, Default)]
pub struct DropTargets {
    pub host_generation: u64,
    pub revision: u64,
    /// At most the two panel list backgrounds. Rows never implicitly navigate or retarget.
    pub targets: Vec<FileDropTarget>,
}
pub type SharedDropTargets = Arc<Mutex<DropTargets>>;

impl DropTargets {
    pub fn validate(&self) -> Result<(), FsError> {
        if self.targets.len() > 2 {
            return Err(failure("At most two panel drop targets are permitted."));
        }
        let mut bytes = 0usize;
        for (index, target) in self.targets.iter().enumerate() {
            validate_target(target)?;
            bytes = bytes.saturating_add(target.directory.as_os_str().as_encoded_bytes().len());
            if self.host_generation == 0
                || self.revision == 0
                || bytes > crate::MAX_FILE_CLIPBOARD_BYTES
                || self.targets[..index].iter().any(|earlier| {
                    earlier.pane == target.pane || earlier.rect.overlaps(target.rect)
                })
            {
                return Err(failure(
                    "Drop targets must be current, distinct and non-overlapping.",
                ));
            }
        }
        Ok(())
    }
    pub fn target_at(&self, host_generation: u64, x: i32, y: i32) -> Option<FileDropTarget> {
        if self.host_generation != host_generation || self.validate().is_err() {
            return None;
        }
        self.targets
            .iter()
            .find(|target| target.rect.contains(x, y))
            .cloned()
    }
}

#[derive(Clone, Debug)]
pub struct FileDropRequest {
    pub files: FileClipboard,
    pub target: FileDropTarget,
    pub targets_revision: u64,
}
impl FileDropRequest {
    pub fn validate(&self) -> Result<(), FsError> {
        self.files.validate()?;
        validate_target(&self.target)?;
        if self.files.intent != FileClipboardIntent::Copy || self.targets_revision == 0 {
            return Err(failure(
                "Drag/drop supports copying only; source files are retained.",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragFinish {
    /// Target accepted the copy offer. The target application owns its eventual operation.
    Copied,
    /// Internal Move was admitted, or a native target reported optimized Move.
    /// Native feedback is not content verification or source-deletion authority.
    Moved,
    /// A native target requested unoptimized Move cleanup without trustworthy
    /// deletion authority. The source is retained; the target may hold a copy.
    SourceRetained,
    /// The target accepted a native drop without trustworthy completion feedback.
    /// Neither source retention nor a completed Move can be asserted.
    Unconfirmed,
    Cancelled,
    Unavailable,
}

#[derive(Clone, Debug)]
pub enum DragDropEvent {
    Hover {
        host_generation: u64,
        pane: Option<PaneId>,
    },
    Ready {
        host_generation: u64,
        supported: bool,
    },
    Prepared {
        id: DragId,
        item_count: usize,
    },
    InternalPreview {
        id: DragId,
        revision: DragPreviewRevision,
        /// None means unavailable or invalid, never an optimistic Copy guess.
        intent: Option<TransferIntent>,
    },
    Finished {
        id: DragId,
        outcome: DragFinish,
    },
    DropAccepted {
        token: DropToken,
        /// The exact internal source gesture; absent for native external drops.
        drag: Option<DragId>,
        pane: PaneId,
        item_count: usize,
        destination: Arc<PathBuf>,
        intent: TransferIntent,
        /// Internal drops freeze this at release; external drops choose it at admission.
        verification: Option<TransferVerification>,
    },
    Failed {
        /// The failed gesture. `None` reports a failure outside any gesture: a native
        /// session that failed to open or was lost (the engine then publishes
        /// `Ready { supported: false }` for that host), or a refused incoming native
        /// drop, after which the session stays usable (see [`FileDropSink::reject`]).
        /// Only `Ready { supported: false }` revokes native drag support.
        id: Option<DragId>,
        error: FsError,
    },
}

/// Fixed refusal report of a native drop target (see [`FileDropSink::reject`]): the
/// destination under the cursor changed since the Copy status was shown.
pub const DROP_DESTINATION_CHANGED: &str = "The drop destination changed.";
/// Fixed refusal report of a native drop target (see [`FileDropSink::reject`]): the drag
/// source did not deliver its file list.
pub const DROP_SOURCE_LIST_MISSING: &str = "The drag source did not provide its file list.";

/// Native callbacks may acknowledge Copy only after this reserves copy-queue capacity.
/// Implementations retain the immutable request and never wait for the UI or perform file I/O.
pub trait FileDropSink: Send + Sync + 'static {
    fn try_accept(&self, request: FileDropRequest) -> Result<(), FsError>;
    /// Latest state only; implementations coalesce before delivering it to the UI.
    fn hover(&self, _pane: Option<PaneId>) {}
    /// Reports why an incoming native drop released after a Copy cursor was refused:
    /// its payload could not be read, its destination or modifiers changed since the
    /// cursor was shown, or [`Self::try_accept`] failed. The native target still
    /// answers "no effect"; this only tells the user why, in fixed text that never
    /// repeats source-supplied descriptions. Called on the native callback thread:
    /// implementations record a bounded report (the engine keeps only the latest
    /// undelivered one) and never wait, block or perform file I/O. The default
    /// ignores the report.
    fn reject(&self, _error: FsError) {}
}

pub trait FileDragDropService: Send + Sync + 'static {
    /// Created, pumped and destroyed on one bounded native actor thread.
    fn open_session(
        &self,
        host: DragHost,
        targets: SharedDropTargets,
        sink: Arc<dyn FileDropSink>,
    ) -> Result<Box<dyn FileDragDropSession>, FsError>;
}

/// Ends a [`FileDragDropSession::wait_pending`] from any thread; see
/// [`FileDragDropSession::waker`].
pub type FileDragDropWaker = Arc<dyn Fn() + Send + Sync>;

/// Thread-affine native objects must not be moved to another worker after construction.
/// Only the [`FileDragDropWaker`] returned by [`waker`](Self::waker) may be used on
/// other threads.
pub trait FileDragDropSession {
    fn service_pending(&mut self) -> Result<(), FsError>;
    /// Block this worker until native input may need [`service_pending`](Self::service_pending),
    /// the session's [`waker`](Self::waker) was called, or `timeout` elapsed. A wake made
    /// at any time after the previous wait returned, including before this call started,
    /// ends this wait at once. A session may shorten the wait to deadlines of its own,
    /// and may serve input it already holds instead of waiting. Returns can be early or
    /// spurious: callers call `service_pending` after every return and re-check their
    /// own state.
    ///
    /// The default parks the thread for at most `timeout` and wakes for nothing else.
    fn wait_pending(&mut self, timeout: Duration) -> Result<(), FsError> {
        std::thread::park_timeout(timeout);
        Ok(())
    }
    /// A waker that ends the current or the next [`wait_pending`](Self::wait_pending) of
    /// this session, or None (the default) when only the timeout ends that wait; such a
    /// session needs short timeouts to stay responsive. Any thread may keep and call the
    /// waker any number of times (calls may coalesce into one wakeup). A call never
    /// blocks, and has no effect once the session was dropped.
    fn waker(&self) -> Option<FileDragDropWaker> {
        None
    }
    /// May run a modal native message loop on this worker. Cancellation and target snapshots
    /// remain observable during that loop. Windows may offer Copy|Move; Linux outgoing
    /// transport offers Copy only. Link is unsupported. The receiver owns external work;
    /// completion feedback never authorizes this source to delete files. Returned outcomes
    /// report receiver feedback, not independent verification of its filesystem operations.
    fn start_drag(
        &mut self,
        files: &FileClipboard,
        cancel: &Cancellation,
    ) -> Result<DragFinish, FsError>;
}

fn validate_target(target: &FileDropTarget) -> Result<(), FsError> {
    let raw = target.directory.as_os_str().as_encoded_bytes();
    if !target.directory.is_absolute()
        || raw.contains(&0)
        || raw.len() > crate::MAX_FILE_CLIPBOARD_BYTES
        || target.rect.width == 0
        || target.rect.height == 0
    {
        return Err(failure(
            "A drop target requires a bounded absolute native path and visible rectangle.",
        ));
    }
    Ok(())
}
fn failure(message: &str) -> FsError {
    FsError::from_io(
        "file drag/drop",
        Path::new(""),
        io::Error::new(io::ErrorKind::InvalidInput, message),
    )
}
