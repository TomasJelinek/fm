//! Captured native inputs for the read-only file comparator.
use crate::{DirectoryIndex, Entry, SelectionSnapshot};
use std::{path::PathBuf, sync::Arc};

#[derive(Clone, Debug)]
pub enum FileCompareSource {
    /// No safe, unambiguous native file is available from this panel.
    /// The comparator offers manual selection instead of guessing a row.
    Unspecified,
    Focused(PathBuf),
    Selected {
        entries: Arc<[Entry]>,
        view: Arc<DirectoryIndex>,
        selection: Arc<SelectionSnapshot>,
    },
    /// Resolve the other side's single file name against this captured visible
    /// native listing on a worker. Never substitute the opposite cursor.
    SameName {
        directory: PathBuf,
        entries: Arc<[Entry]>,
        view: Arc<DirectoryIndex>,
    },
}

#[derive(Clone, Debug)]
pub struct FileCompareLaunch {
    /// At most two cheap snapshots. Selection expansion belongs to the worker.
    pub sources: Vec<FileCompareSource>,
    pub disabled_plugins: Arc<[String]>,
}
