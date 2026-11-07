//! Captured native inputs for the read-only file comparator.
use crate::{DirectoryIndex, Entry, SelectionSnapshot};
use std::{path::PathBuf, sync::Arc};

#[derive(Clone, Debug)]
pub enum FileCompareSource {
    Focused(PathBuf),
    Selected {
        entries: Arc<[Entry]>,
        view: Arc<DirectoryIndex>,
        selection: Arc<SelectionSnapshot>,
    },
}

#[derive(Clone, Debug)]
pub struct FileCompareLaunch {
    /// At most two cheap snapshots. Selection expansion belongs to the worker.
    pub sources: Vec<FileCompareSource>,
    pub disabled_plugins: Arc<[String]>,
}
