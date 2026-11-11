//! External editing from a captured inline-tree occurrence, never a display path.
use crate::{Cancellation, FsError, TreeProjection, TreeRequestId, TreeRowId, TreeViewStamp};
use std::{path::PathBuf, sync::Arc};

/// F4 retains the focused row independently of selection. Admission requires the
/// exact acknowledged projection and an ordinary native file at that occurrence.
#[derive(Clone, Debug)]
pub struct TreeEditorRequest {
    pub view: Arc<TreeProjection>,
    pub request: TreeRequestId,
    pub row: TreeRowId,
    pub cancel: Cancellation,
}

/// Backend-owned provider authority for one native regular file. Identity accessors
/// are cached; resolve and final destruction belong to backend workers. The editor
/// service still revalidates the live regular file before spawning an external app.
pub trait CapturedEditorFile: std::fmt::Debug + Send + Sync {
    fn stamp(&self) -> TreeViewStamp;
    fn row(&self) -> TreeRowId;
    fn resolve(&self, cancel: &Cancellation) -> Result<PathBuf, FsError>;
}
