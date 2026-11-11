//! Backend action admission for an exact acknowledged tree selection.
use super::{CapturedTarget, Session, check_cancel};
use crate::providers::{ArchiveTarget, MountedLocation, ProviderError, ProviderTarget};
use midnight_core::{
    ArchiveMount, Cancellation, NodeRef, SelectionRevision, TreeProjection, TreeRowId,
    TreeSelectionSnapshot,
};
use std::{mem::size_of, path::PathBuf, sync::Arc};

// Match existing clipboard, checksum and command capture counts. These are batch
// admission budgets, not new panel-size or individual native path-length limits.
const MAX_TARGETS: usize = 4096;
const MAX_OWNED_BYTES: usize = 4 * 1024 * 1024;

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
        check_cancel(cancel)?;
        self.check_view(view)?;
        if revision != self.selection.revision
            || !Arc::ptr_eq(selection, &self.selection.snapshot)
            || !Arc::ptr_eq(selection.basis(), view)
        {
            return Err(ProviderError::StaleReference);
        }
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
        check_cancel(cancel)?;
        self.check_view(view)?;
        if revision != self.selection.revision
            || !Arc::ptr_eq(selection, &self.selection.snapshot)
            || !Arc::ptr_eq(selection.basis(), view)
        {
            return Err(ProviderError::StaleReference);
        }
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
