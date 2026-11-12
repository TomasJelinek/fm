//! Scope-bound inline-tree requests and atomic view/selection publication.
use crate::{
    ArchiveLocation, EntryId, NavigationId, PanelFilter, SelectionRevision, SelectionSnapshot,
    SortSpec, TreeProjection, TreeRequestId, TreeRootSnapshot, TreeRowId, TreeSelectionRequest,
    TreeSelectionSnapshot, TreeViewStamp,
};
use std::{collections::HashSet, path::PathBuf, sync::Arc};

/// A diagnostic owns a fresh bounded UTF-8 allocation, never a truncated large String.
#[derive(Clone, Debug)]
pub struct TreeDiagnostic {
    summary: Arc<str>,
    details: Option<Arc<dyn crate::GeneratedTextSource>>,
}

impl PartialEq for TreeDiagnostic {
    fn eq(&self, other: &Self) -> bool {
        self.summary == other.summary
            && match (&self.details, &other.details) {
                (None, None) => true,
                (Some(left), Some(right)) => Arc::ptr_eq(left, right),
                _ => false,
            }
    }
}
impl Eq for TreeDiagnostic {}

impl TreeDiagnostic {
    pub const MAX_BYTES: usize = 4096;

    pub fn new(message: &str) -> Self {
        let mut end = message.len().min(Self::MAX_BYTES);
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        Self {
            summary: Arc::from(&message[..end]),
            details: None,
        }
    }

    /// Retain a backend-frozen full diagnostic separately from the bounded summary.
    /// Equality compares retained snapshot identity, never reads report content.
    /// Construction, reads and final source destruction belong on backend workers.
    pub fn with_details(message: &str, details: Arc<dyn crate::GeneratedTextSource>) -> Self {
        Self {
            details: Some(details),
            ..Self::new(message)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.summary
    }

    pub fn details(&self) -> Option<&Arc<dyn crate::GeneratedTextSource>> {
        self.details.as_ref()
    }
}

/// The already accepted root selection; an empty selection needs no fabricated native index.
#[derive(Clone, Debug)]
pub enum TreeRootSelection {
    Empty,
    Native(Arc<SelectionSnapshot>),
    Archive(Arc<HashSet<EntryId>>),
}

impl TreeRootSelection {
    /// Validate on a backend worker before migrating selection into a tree session.
    pub fn validate_for(&self, root: &TreeRootSnapshot) -> Result<(), &'static str> {
        root.validate()?;
        match self {
            Self::Empty => Ok(()),
            Self::Native(selection) => {
                if !Arc::ptr_eq(selection.basis(), &root.index)
                    || root
                        .entries
                        .iter()
                        .any(|entry| entry.virtual_node.is_some())
                {
                    return Err("tree root selection does not match its exact native index");
                }
                Ok(())
            }
            Self::Archive(selected) => {
                if selected.len() > root.entries.len()
                    || selected.capacity() > TreeProjection::MAX_ROWS * 2
                    || root
                        .entries
                        .iter()
                        .any(|entry| entry.virtual_node.is_none())
                    || selected
                        .iter()
                        .any(|entry| !root.index.by_id.contains_key(entry))
                {
                    return Err("tree root selection has foreign or excessive archive identities");
                }
                Ok(())
            }
        }
    }
}

/// Visible/overscan occurrence hints. The actor rechecks the exact captured view before use.
#[derive(Clone, Debug)]
pub struct TreeViewport {
    rows: Arc<[TreeRowId]>,
}

impl TreeViewport {
    pub const MAX_ROWS: usize = 256;

    pub fn new(view: &TreeProjection, rows: Vec<TreeRowId>) -> Result<Self, &'static str> {
        if rows.len() > Self::MAX_ROWS {
            return Err("tree viewport exceeds its probe budget");
        }
        let mut unique = HashSet::with_capacity(rows.len());
        for row in &rows {
            if !view.index.row_positions.contains_key(row) || !unique.insert(*row) {
                return Err("tree viewport has duplicate or foreign occurrences");
            }
        }
        Ok(Self { rows: rows.into() })
    }

    pub fn rows(&self) -> &[TreeRowId] {
        &self.rows
    }
}

/// A view and its fully rebased selection are accepted together, never in separate events.
#[derive(Clone, Debug)]
pub struct TreePublication {
    pub view: Arc<TreeProjection>,
    pub selection_revision: SelectionRevision,
    pub selection: Arc<TreeSelectionSnapshot>,
}

impl TreePublication {
    pub fn new(
        view: Arc<TreeProjection>,
        selection_revision: SelectionRevision,
        selection: Arc<TreeSelectionSnapshot>,
    ) -> Result<Self, &'static str> {
        if !Arc::ptr_eq(&view, selection.basis()) {
            return Err("tree publication selection does not use its exact view");
        }
        Ok(Self {
            view,
            selection_revision,
            selection,
        })
    }

    /// Fence first adoption against a newer pending or accepted flat selection.
    /// A failed check leaves the flat panel intact; the actor must retry migration.
    pub fn can_adopt(
        &self,
        root: &TreeRootSnapshot,
        requested_revision: SelectionRevision,
    ) -> bool {
        self.view.root.same_snapshot(root)
            && self.selection_revision == requested_revision
            && Arc::ptr_eq(&self.view, self.selection.basis())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeBranchAction {
    Probe,
    Expand,
    Collapse,
    Refresh,
}

/// Transient literal prefix search over loaded rows, never a recursive provider query.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TreeSearchOptions {
    pub query: String,
    pub filter_matches: bool,
}

impl TreeSearchOptions {
    /// A command admission budget, independent of filename and filesystem path lengths.
    pub const MAX_QUERY_BYTES: usize = 4096;

    pub fn validate_bounds(&self) -> Result<(), &'static str> {
        if self.query.len() > Self::MAX_QUERY_BYTES || self.query.contains('\0') {
            return Err("Tree search query exceeds its budget or contains NUL.");
        }
        Ok(())
    }
}

/// Presentation choices for retained tree rows; no provider operations are implied.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TreeProjectionOptions {
    pub sort: SortSpec,
    pub filter: PanelFilter,
    pub search: TreeSearchOptions,
}

/// Reprojection requires the exact acknowledged view and settled selection.
#[derive(Clone, Debug)]
pub struct TreeReprojectRequest {
    pub view: Arc<TreeProjection>,
    pub request: TreeRequestId,
    pub selection_revision: SelectionRevision,
    pub selection: Arc<TreeSelectionSnapshot>,
    pub options: TreeProjectionOptions,
}

/// Reconcile previously loaded native branches in the exact acknowledged session.
#[derive(Clone, Debug)]
pub struct TreeRefreshRequest {
    pub view: Arc<TreeProjection>,
    pub request: TreeRequestId,
}

/// Nest under `Command::Tree(Box<TreeCommand>)` when the production actor is connected.
/// Every view must be the exact acknowledged Arc; equal stamps alone are insufficient.
#[derive(Clone, Debug)]
pub enum TreeCommand {
    CaptureDropDirectory {
        id: crate::DragId,
        view: Arc<TreeProjection>,
        request: TreeRequestId,
        row: TreeRowId,
    },
    Clipboard(crate::TreeClipboardRequest),
    Edit(crate::TreeEditorRequest),
    Checksums(crate::TreeChecksumRequest),
    Transfer(crate::TreeTransferRequest),
    Delete(crate::TreeDeleteRequest),
    Sizes(crate::TreeSizeCommand),
    /// Root-ready and this request may arrive in either order. Both root Arcs must match.
    Enable {
        root: TreeRootSnapshot,
        selection_revision: SelectionRevision,
        selected: TreeRootSelection,
    },
    Commit(Arc<TreeProjection>),
    Branch {
        view: Arc<TreeProjection>,
        request: TreeRequestId,
        row: TreeRowId,
        action: TreeBranchAction,
    },
    Viewport {
        view: Arc<TreeProjection>,
        rows: TreeViewport,
    },
    Select(TreeSelectionRequest),
    Reproject(TreeReprojectRequest),
    RefreshLoaded(TreeRefreshRequest),
    Navigate {
        view: Arc<TreeProjection>,
        request: TreeRequestId,
        row: TreeRowId,
        /// A fresh destination generation from the existing pane navigation allocator.
        navigation: NavigationId,
    },
}

/// Navigation may retain an existing archive mount; retire archive payloads on workers.
/// Copy events also retain opaque sources. Display projections and selection own no providers.
#[derive(Clone, Debug)]
pub enum TreeNavigationTarget {
    Native(PathBuf),
    Archive(ArchiveLocation),
}

/// Nest under `Event::Tree(Box<TreeEvent>)` when the production presenter is connected.
#[derive(Clone, Debug)]
pub enum TreeEvent {
    DeleteCaptured {
        stamp: TreeViewStamp,
        request: TreeRequestId,
        result: Result<crate::TreeDeleteCapture, TreeDiagnostic>,
    },
    ChecksumsCaptured {
        stamp: TreeViewStamp,
        request: TreeRequestId,
        result: Result<crate::TreeChecksumCapture, TreeDiagnostic>,
    },
    EditorCaptured {
        stamp: TreeViewStamp,
        request: TreeRequestId,
        result: Result<Arc<dyn crate::CapturedEditorFile>, TreeDiagnostic>,
    },
    DropDirectoryCaptured {
        stamp: TreeViewStamp,
        request: TreeRequestId,
        id: crate::DragId,
        result: Result<Arc<dyn crate::CapturedDropDirectory>, TreeDiagnostic>,
    },
    Clipboard {
        stamp: TreeViewStamp,
        request: TreeRequestId,
        result: Result<Arc<dyn crate::CapturedClipboardFiles>, TreeDiagnostic>,
    },
    /// A native observation requests maintenance of this accepted tree session.
    Changed {
        stamp: TreeViewStamp,
    },
    /// Watch enrollment failed after a successful Commit; do not retry that Commit.
    WatchFailed {
        stamp: TreeViewStamp,
        error: TreeDiagnostic,
    },
    Refreshed {
        stamp: TreeViewStamp,
        request: TreeRequestId,
        result: Result<(), TreeDiagnostic>,
    },
    RefreshRejected {
        stamp: TreeViewStamp,
        request: TreeRequestId,
        error: TreeDiagnostic,
        retryable: bool,
    },
    Transfer {
        stamp: TreeViewStamp,
        request: TreeRequestId,
        result: Result<crate::TreeTransferCapture, TreeDiagnostic>,
    },
    Sizes(crate::TreeSizeEvent),
    Published(TreePublication),
    Reprojected {
        previous: TreeViewStamp,
        request: TreeRequestId,
        options: TreeProjectionOptions,
        publication: TreePublication,
    },
    ReprojectFailed {
        stamp: TreeViewStamp,
        request: TreeRequestId,
        /// True only for transient command admission or stale-view/selection failures.
        /// Validation, projection and resource failures require a changed request.
        retryable: bool,
        error: TreeDiagnostic,
    },
    Unavailable {
        root: TreeRootSnapshot,
        error: TreeDiagnostic,
    },
    Rejected {
        stamp: TreeViewStamp,
        request: TreeRequestId,
        row: TreeRowId,
        error: TreeDiagnostic,
    },
    SelectionChanged {
        view: Arc<TreeProjection>,
        revision: SelectionRevision,
        selection: Arc<TreeSelectionSnapshot>,
    },
    SelectionFailed {
        stamp: TreeViewStamp,
        revision: SelectionRevision,
        error: TreeDiagnostic,
    },
    Navigated {
        stamp: TreeViewStamp,
        request: TreeRequestId,
        row: TreeRowId,
        navigation: NavigationId,
        target: TreeNavigationTarget,
    },
}
