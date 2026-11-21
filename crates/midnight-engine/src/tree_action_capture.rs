//! Backend action admission for an exact acknowledged tree selection.
use super::{CapturedTarget, Session, check_cancel};
use crate::providers::{ArchiveTarget, MountedLocation, ProviderError, ProviderTarget};
use midnight_core::{
    ArchiveMount, Cancellation, EntryKind, FsError, NodeRef, SelectionRevision, TreeProjection,
    TreeRowId, TreeSelectionSnapshot,
};
use std::{
    collections::HashSet,
    io,
    mem::size_of,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

/// A capture takes every occurrence its projection can select: no more targets than the
/// projection has rows (a selection never holds more), and no more newly owned target
/// and context memory than the projection itself may hold. These follow the tree's own
/// bounds rather than a count of their own (4,096 until 21 November 2026); they are not
/// panel-size or native path-length limits. Each consumer still bounds what it holds per
/// target: Copy and Move the native roots they keep open, Delete its pinned roots,
/// checksums, Custom Commands and metadata edits their own targets.
pub(super) const MAX_TARGETS: usize = TreeProjection::MAX_ROWS;
pub(super) const MAX_OWNED_BYTES: usize = TreeProjection::MAX_BYTES;

/// Exact display occurrences admitted together on a backend worker. Shared view,
/// selection and provider buffers retain their existing tree/provider budgets;
/// this batch charges newly owned target metadata and context bookkeeping.
///
/// Targets retain their registrations across later pane navigation. They provide
/// context, not native mutation proof or an immutable archive source revision.
/// Consumers must retain the captures while establishing operation authority.
pub(crate) struct CapturedSelection {
    pub(crate) view: Arc<TreeProjection>,
    // Retain the exact admitted snapshot until backend consumers retire it;
    // actions use the captured targets instead of rereading current selection.
    pub(crate) _selection: Arc<TreeSelectionSnapshot>,
    pub(crate) targets: Vec<CapturedOccurrence>,
}

/// An occurrence is not a unique filesystem object. Preserve selection order and
/// aliases/overlapping ancestors here; operation-specific preflight must decide
/// overlap policy using provider identity and ancestry, never caption prefixes.
pub(crate) struct CapturedOccurrence {
    pub(crate) row: TreeRowId,
    pub(crate) captured: CapturedTarget,
}

impl Session {
    /// Shared exact-selection admission, including consumers with no file arguments.
    pub(crate) fn check_selection(
        &self,
        view: &Arc<TreeProjection>,
        revision: SelectionRevision,
        selection: &Arc<TreeSelectionSnapshot>,
        cancel: &Cancellation,
    ) -> Result<(), ProviderError> {
        check_cancel(cancel)?;
        self.check_view(view)?;
        if revision != self.selection.revision
            || !Arc::ptr_eq(selection, &self.selection.snapshot)
            || !Arc::ptr_eq(selection.basis(), view)
        {
            return Err(ProviderError::StaleReference);
        }
        Ok(())
    }

    pub(crate) fn capture_sizes(
        &mut self,
        request: &midnight_core::TreeSizeRequest,
        cancel: &Cancellation,
    ) -> Result<CapturedSelection, ProviderError> {
        check_cancel(cancel)?;
        self.check_view(&request.view)?;
        if request.request.0 == 0
            || request.request.0 <= self.last_request.0
            || request.selection_revision != self.selection.revision
            || !Arc::ptr_eq(&request.selection, &self.selection.snapshot)
            || !Arc::ptr_eq(request.selection.basis(), &request.view)
        {
            return Err(ProviderError::StaleReference);
        }
        let rows = request
            .view
            .rows
            .iter()
            .enumerate()
            .filter(|(position, row)| {
                let requested = match request.scope {
                    midnight_core::TreeSizeScope::Single(id) => row.id == id,
                    midnight_core::TreeSizeScope::SelectionOrVisible => {
                        request.selection.is_empty()
                            || request.selection.contains_position(*position)
                    }
                };
                requested
                    && row.selectable
                    && !row.context_only
                    && matches!(
                        row.entry.kind,
                        midnight_core::EntryKind::Directory
                            | midnight_core::EntryKind::DirectoryLink
                    )
            })
            .map(|(_, row)| (row.id, row.depth));
        let count = rows.clone().count();
        if matches!(request.scope, midnight_core::TreeSizeScope::Single(_)) && count != 1 {
            return Err(ProviderError::Unsupported);
        }
        let targets = self.capture_occurrences(&request.view, count, rows, cancel)?;
        self.last_request = request.request;
        Ok(CapturedSelection {
            view: request.view.clone(),
            _selection: request.selection.clone(),
            targets,
        })
    }
    /// Capture an exact acknowledged publication without publishing partial work.
    /// A nonempty selection always wins over fallback, even a stale fallback.
    /// Cancellation or any target failure releases the entire in-progress batch.
    /// Call and destroy the result on backend workers, not on the UI thread.
    pub(crate) fn capture_selection(
        &self,
        view: &Arc<TreeProjection>,
        revision: SelectionRevision,
        selection: &Arc<TreeSelectionSnapshot>,
        fallback: Option<TreeRowId>,
        cancel: &Cancellation,
    ) -> Result<CapturedSelection, ProviderError> {
        self.check_selection(view, revision, selection, cancel)?;
        let fallback = if selection.is_empty() {
            let row = fallback.ok_or(ProviderError::Unsupported)?;
            self.visible_branch(row)?;
            Some(row)
        } else {
            None
        };
        let count = selection.len().max(usize::from(fallback.is_some()));
        let rows = view
            .rows
            .iter()
            .enumerate()
            .filter(|(position, row)| {
                selection.contains_position(*position) || fallback == Some(row.id)
            })
            .map(|(_, row)| (row.id, row.depth));
        let targets = self.capture_occurrences(view, count, rows, cancel)?;
        Ok(CapturedSelection {
            view: view.clone(),
            _selection: selection.clone(),
            targets,
        })
    }

    pub(crate) fn capture_dragged(
        &self,
        view: &Arc<TreeProjection>,
        revision: SelectionRevision,
        selection: &Arc<TreeSelectionSnapshot>,
        dragged: TreeRowId,
        cancel: &Cancellation,
    ) -> Result<CapturedSelection, ProviderError> {
        self.check_selection(view, revision, selection, cancel)?;
        let position = view
            .index
            .row_positions
            .get(&dragged)
            .copied()
            .ok_or(ProviderError::StaleReference)?;
        if selection.contains_position(position) {
            return self.capture_selection(view, revision, selection, Some(dragged), cancel);
        }
        let row = &view.rows[position];
        let targets =
            self.capture_occurrences(view, 1, std::iter::once((dragged, row.depth)), cancel)?;
        Ok(CapturedSelection {
            view: view.clone(),
            _selection: selection.clone(),
            targets,
        })
    }

    /// Shared all-or-nothing target admission for already validated action scopes.
    /// Borrowed iterators preserve display order without an intermediate row allocation.
    fn capture_occurrences(
        &self,
        view: &Arc<TreeProjection>,
        count: usize,
        rows: impl Iterator<Item = (TreeRowId, usize)>,
        cancel: &Cancellation,
    ) -> Result<Vec<CapturedOccurrence>, ProviderError> {
        check_cancel(cancel)?;
        if count > MAX_TARGETS {
            return Err(ProviderError::ResourceLimited);
        }
        let mut targets = Vec::new();
        targets
            .try_reserve_exact(count)
            .map_err(|_| ProviderError::ResourceLimited)?;
        let mut bytes = size_of::<CapturedSelection>().saturating_add(
            targets
                .capacity()
                .saturating_mul(size_of::<CapturedOccurrence>()),
        );
        for (row, depth) in rows {
            check_cancel(cancel)?;
            let captured = self.capture_target(view, row, cancel)?;
            bytes = bytes
                .saturating_add(context_charge(depth)?)
                .saturating_add(target_charge(captured.target.value()?));
            if bytes > MAX_OWNED_BYTES {
                return Err(ProviderError::ResourceLimited);
            }
            check_cancel(cancel)?;
            targets.push(CapturedOccurrence { row, captured });
        }
        // Later resolutions may revoke an earlier target's provider context.
        for occurrence in &targets {
            check_cancel(cancel)?;
            occurrence.captured.target.value()?;
        }
        check_cancel(cancel)?;
        Ok(targets)
    }
}

fn context_charge(depth: usize) -> Result<usize, ProviderError> {
    // DirectoryTree retains up to two contexts for each ancestor (source and
    // attached container), including the hidden root, then the target itself.
    // Reserve the Vec's minimum four slots and doubling growth, plus its Arc
    // header. MountedLocation/NodeRef clones share address/token/mount storage;
    // their inline reference bookkeeping is charged in each reserved slot.
    let slots = depth
        .saturating_add(1)
        .saturating_mul(2)
        .saturating_add(1)
        .max(4)
        .checked_next_power_of_two()
        .ok_or(ProviderError::ResourceLimited)?;
    Ok(slots
        .saturating_mul(size_of::<(MountedLocation, NodeRef)>())
        .saturating_add(size_of::<Vec<(MountedLocation, NodeRef)>>())
        .saturating_add(2 * size_of::<usize>()))
}

fn target_charge(target: &ProviderTarget) -> usize {
    match target {
        ProviderTarget::Native(path) => path.capacity(),
        ProviderTarget::Archive(target) => archive_charge(target),
    }
}

fn archive_charge(target: &ArchiveTarget) -> usize {
    let location = &target.location;
    let address = &location.address;
    // resolve_target_with_context adds a LeasedArchive Arc and a container-path
    // clone. Charge that wrapper too; the source mount and raw-name Arcs remain
    // shared with the bounded provider records, not copied for every occurrence.
    let mut bytes = size_of::<ArchiveTarget>()
        .saturating_add(size_of::<(Arc<dyn ArchiveMount>, Arc<()>, PathBuf)>())
        .saturating_add(2 * size_of::<usize>())
        .saturating_add(address.container.capacity().saturating_mul(2))
        .saturating_add(address.provider.capacity())
        .saturating_add(address.source_version.capacity())
        .saturating_add(
            address
                .components
                .capacity()
                .saturating_mul(size_of::<Vec<u8>>()),
        )
        .saturating_add(
            location
                .trail
                .capacity()
                .saturating_mul(size_of::<(u64, String)>()),
        );
    for component in &address.components {
        bytes = bytes.saturating_add(component.capacity());
    }
    for (_, caption) in &location.trail {
        bytes = bytes.saturating_add(caption.capacity());
    }
    for node in [&target.member, &target.original].into_iter().flatten() {
        bytes = bytes.saturating_add(node.display_name.capacity());
    }
    bytes
}

/// Shared policy for captured tree actions. Each consumer supplies its own
/// operation label and message; the rules themselves must not diverge.
pub(crate) fn capture_failure(operation: &'static str, error: ProviderError) -> FsError {
    let kind = match error {
        ProviderError::Cancelled => io::ErrorKind::Interrupted,
        ProviderError::Unsupported => io::ErrorKind::Unsupported,
        _ => io::ErrorKind::InvalidData,
    };
    FsError::from_io(operation, Path::new(""), io::Error::new(kind, error))
}

/// A captured native file or folder occurrence: absolute, NUL-free and ending
/// in a normal name, never parent navigation or a volume root.
pub(crate) fn validate_native_leaf(
    path: &Path,
    message: &'static str,
) -> Result<(), ProviderError> {
    if !path.is_absolute()
        || path.as_os_str().as_encoded_bytes().contains(&0)
        || !matches!(path.components().next_back(), Some(Component::Normal(_)))
        || path.file_name().is_none()
    {
        return Err(ProviderError::InvalidResponse(message));
    }
    Ok(())
}

/// The nearest selected ancestor covering `row` in the captured view. Overlap
/// through a link or another non-directory ancestor is refused with `overlap`:
/// such roots must be handled separately, never as one mutable subtree.
pub(crate) fn covering_ancestor(
    view: &TreeProjection,
    row: TreeRowId,
    selected: &HashSet<TreeRowId>,
    overlap: &'static str,
) -> Result<Option<TreeRowId>, ProviderError> {
    let lookup = |id: TreeRowId| {
        view.index
            .row_positions
            .get(&id)
            .and_then(|position| view.rows.get(*position))
            .ok_or(ProviderError::StaleReference)
    };
    let parent = lookup(row)?.parent;
    covering_chain(parent, selected, overlap, |id| {
        let row = lookup(id)?;
        Ok((row.entry.kind, row.parent))
    })
}

/// Ancestor walk behind [`covering_ancestor`], over any occurrence lookup.
pub(crate) fn covering_chain(
    mut parent: Option<TreeRowId>,
    selected: &HashSet<TreeRowId>,
    overlap: &'static str,
    mut lookup: impl FnMut(TreeRowId) -> Result<(EntryKind, Option<TreeRowId>), ProviderError>,
) -> Result<Option<TreeRowId>, ProviderError> {
    let mut crossed_boundary = false;
    for _ in 0..=TreeProjection::MAX_DEPTH {
        let Some(id) = parent else { return Ok(None) };
        let (kind, next) = lookup(id)?;
        crossed_boundary |= kind != EntryKind::Directory;
        if selected.contains(&id) {
            if crossed_boundary {
                return Err(ProviderError::Failed(overlap.into()));
            }
            return Ok(Some(id));
        }
        parent = next;
    }
    Err(ProviderError::StaleReference)
}
