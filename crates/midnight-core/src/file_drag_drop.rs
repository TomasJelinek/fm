//! Copy-only native drag/drop contracts. Paths and admission stay on the backend.
use crate::{Cancellation, FileClipboard, FileClipboardIntent, FsError, NavigationId, PaneId};
use std::{
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DragId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DropToken(pub u64);

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
    Finished {
        id: DragId,
        outcome: DragFinish,
    },
    DropAccepted {
        token: DropToken,
        pane: PaneId,
        item_count: usize,
        destination: Arc<PathBuf>,
    },
    Failed {
        id: Option<DragId>,
        error: FsError,
    },
}

/// Native callbacks may acknowledge Copy only after this reserves copy-queue capacity.
/// Implementations retain the immutable request and never wait for the UI or perform file I/O.
pub trait FileDropSink: Send + Sync + 'static {
    fn try_accept(&self, request: FileDropRequest) -> Result<(), FsError>;
    /// Latest state only; implementations coalesce before delivering it to the UI.
    fn hover(&self, _pane: Option<PaneId>) {}
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

/// Thread-affine native objects must not be moved to another worker after construction.
pub trait FileDragDropSession {
    fn service_pending(&mut self) -> Result<(), FsError>;
    /// May run a modal native message loop on this worker. Cancellation and target snapshots
    /// remain observable during that loop. Only Copy may be offered, never Move or Link.
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
