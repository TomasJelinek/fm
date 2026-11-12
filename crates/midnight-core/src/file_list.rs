//! Immutable Make File List captures and backend job messages.
use crate::*;
use std::{collections::HashSet, fmt, io, path::PathBuf, sync::Arc};

/// Readable immutable generated text. Implementations and all reads live on backend workers.
pub trait GeneratedTextSource: fmt::Debug + Send + Sync {
    fn length(&self) -> u64;
    fn read_at(&self, offset: u64, output: &mut [u8]) -> io::Result<usize>;
}

#[derive(Clone, Debug)]
pub enum FileListSource {
    Native {
        entries: Arc<[Entry]>,
        view: Arc<DirectoryIndex>,
        selection: Arc<SelectionSnapshot>,
        fallback: Option<EntryId>,
    },
    Archive {
        entries: Arc<[Entry]>,
        view: Arc<DirectoryIndex>,
        selected: Arc<HashSet<EntryId>>,
        fallback: Option<EntryId>,
    },
    Tree {
        view: Arc<TreeProjection>,
        selection: Arc<TreeSelectionSnapshot>,
        fallback: Option<TreeRowId>,
    },
}

#[derive(Clone, Debug)]
pub struct FileListCapture {
    pub source: FileListSource,
    /// Display prefix only; never an authority for filesystem access.
    pub directory: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileListDestination {
    Preview,
    Clipboard,
    Viewer,
    File { path: PathBuf, append: bool },
}

#[derive(Clone, Debug)]
pub struct FileListRequest {
    pub id: DesktopRequestId,
    pub generation: u64,
    pub capture: Arc<FileListCapture>,
    pub template: String,
    pub destination: FileListDestination,
}

#[derive(Clone, Debug)]
pub struct FileListEvent {
    pub id: DesktopRequestId,
    pub generation: u64,
    pub result: Result<FileListUpdate, FsError>,
}

#[derive(Clone, Debug)]
pub enum FileListUpdate {
    Preview {
        text: String,
        items: u64,
        truncated: bool,
    },
    Generated {
        content: Arc<dyn GeneratedTextSource>,
        items: u64,
    },
    Conflict {
        entry: TransferEntry,
        append: bool,
    },
    Saved {
        path: PathBuf,
        bytes: u64,
        recovery: Vec<PathBuf>,
        warning: Option<FsError>,
    },
}
