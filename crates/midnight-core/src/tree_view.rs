//! Immutable inline-tree presentation and selection. No provider handles cross this boundary.
use crate::{
    ArchiveSelectionTotals, DirectoryIndex, Entry, EntryId, EntryKind, NavigationId, PaneId,
    SelectionMode, SelectionOperation, SelectionRanges, SelectionRevision,
};
use std::{
    collections::{HashMap, HashSet},
    mem::{size_of, size_of_val},
    sync::Arc,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TreeSessionId(pub u64);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TreeRevision(pub u64);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TreeLayoutRevision(pub u64);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TreeRowId(pub u64);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TreeRequestId(pub u64);

macro_rules! checked_tree_counter {
    ($($id:ty),+ $(,)?) => {$ (
        impl $id {
            /// Allocate the next nonzero identity; exhaustion never wraps or reuses a value.
            pub fn checked_next(self) -> Option<Self> {
                self.0.checked_add(1).map(Self)
            }
        }
    )+};
}
checked_tree_counter!(
    TreeSessionId,
    TreeRevision,
    TreeLayoutRevision,
    TreeRowId,
    TreeRequestId
);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TreeViewStamp {
    pub pane: PaneId,
    pub navigation: NavigationId,
    pub session: TreeSessionId,
    pub revision: TreeRevision,
}

/// The exact accepted flat listing from which a tree was seeded, without provider leases.
#[derive(Clone, Debug)]
pub struct TreeRootSnapshot {
    pub pane: PaneId,
    pub navigation: NavigationId,
    pub entries: Arc<[Entry]>,
    pub index: Arc<DirectoryIndex>,
}

impl TreeRootSnapshot {
    /// Bounded admission before retaining a root in a backend command queue.
    /// Accounts for spare index capacity, raw paths, cached text and sort keys.
    pub fn within_payload_budget(&self) -> bool {
        let maximum = TreeProjection::MAX_ROWS;
        self.entries.len() < maximum
            && self.index.by_id.capacity() <= maximum * 2
            && self.index.by_path.capacity() <= maximum * 2
            && self.index.directory_bits.capacity() <= maximum.div_ceil(64) * 2
            && self.index.file_sizes.capacity() <= maximum * 2
            && root_bytes(self) <= TreeProjection::MAX_BYTES
    }

    /// Equal contents are insufficient: both allocations and the pane generation must match.
    pub fn same_snapshot(&self, other: &Self) -> bool {
        self.pane == other.pane
            && self.navigation == other.navigation
            && Arc::ptr_eq(&self.entries, &other.entries)
            && Arc::ptr_eq(&self.index, &other.index)
    }

    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        let count = self.entries.len();
        if count > TreeProjection::MAX_ROWS
            || self.index.by_id.len() != count
            || self.index.directory_bits.len() != count.div_ceil(64)
            || self.index.file_sizes.len() != count
        {
            return Err("tree root listing exceeds its bounds or has a mismatched index");
        }
        let mut native_count = 0;
        for (position, entry) in self.entries.iter().enumerate() {
            if self.index.by_id.get(&entry.id) != Some(&position) {
                return Err("tree root index does not identify its exact listing");
            }
            if let Some(path) = entry.native_path() {
                native_count += 1;
                if !path.is_absolute() || self.index.by_path.get(path) != Some(&entry.id) {
                    return Err("tree root index does not identify its exact native paths");
                }
            }
            let directory = matches!(entry.kind, EntryKind::Directory | EntryKind::DirectoryLink);
            let size = (entry.kind == EntryKind::File)
                .then_some(entry.metadata.size)
                .flatten();
            if self.index.is_directory(entry.id) != directory
                || self.index.file_sizes[position] != size
            {
                return Err("tree root index metadata does not match its listing");
            }
        }
        if self.index.by_path.len() != native_count {
            return Err("tree root index has foreign native paths");
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TreeDisclosure {
    #[default]
    None,
    Unknown,
    Loading,
    Collapsed,
    Expanded,
    Empty,
    Failed,
}

#[derive(Clone, Debug)]
pub struct TreeDisplayRow {
    pub id: TreeRowId,
    pub entry: Entry,
    pub parent: Option<TreeRowId>,
    pub depth: usize,
    pub disclosure: TreeDisclosure,
    pub expanded: bool,
    pub selectable: bool,
    pub navigable: bool,
    pub context_only: bool,
    pub error: Option<String>,
    /// Full immutable backend diagnostic; `error` remains a bounded row summary.
    pub error_details: Option<Arc<dyn crate::GeneratedTextSource>>,
    /// Captured root-action alias; initially also retains seed focus and selection.
    pub original_root_entry: Option<EntryId>,
    pub prefix_end: usize,
    pub search_match: bool,
}

impl TreeDisplayRow {
    /// Shared bulk-selection policy for backend state and immediate UI feedback.
    /// Select All and Files Only replace selection with top-level rows. Invert All changes
    /// only top-level rows and preserves explicitly marked descendants.
    pub fn bulk_selected(&self, operation: SelectionOperation, selected: bool) -> bool {
        if !self.selectable || self.context_only {
            return false;
        }
        if self.depth != 0 {
            match operation {
                SelectionOperation::All | SelectionOperation::FilesOnly => return false,
                SelectionOperation::InvertAll => return selected,
                _ => {}
            }
        }
        operation.selects(
            matches!(
                self.entry.kind,
                EntryKind::Directory | EntryKind::DirectoryLink
            ),
            selected,
        )
    }
}

#[derive(Clone, Debug)]
pub struct TreeDisplayIndex {
    pub row_positions: HashMap<TreeRowId, usize>,
    /// Ordered projection positions of matching, non-context rows, built on the backend.
    pub search_rows: Arc<[usize]>,
    /// Visible root aliases for constant-time focus and initial disclosure adoption.
    /// These map original listing identities, never descendant operation authority.
    pub root_rows: HashMap<EntryId, TreeRowId>,
    /// Rendering/search compatibility only. Never a native operation selection scope.
    pub directory: Arc<DirectoryIndex>,
}

#[derive(Clone, Debug)]
pub struct TreeProjection {
    pub root: TreeRootSnapshot,
    /// Current backend-owned root action identities; admission `root` stays immutable.
    pub root_actions: TreeRootSnapshot,
    pub stamp: TreeViewStamp,
    /// Changes for structural/order/selectability changes, not disclosure/metadata alone.
    pub layout_revision: TreeLayoutRevision,
    pub rows: Arc<[TreeDisplayRow]>,
    pub index: Arc<TreeDisplayIndex>,
}

impl TreeProjection {
    pub const MAX_ROWS: usize = 10_000;
    pub const MAX_BYTES: usize = 16 * 1024 * 1024;
    pub const MAX_DEPTH: usize = 64;

    /// Backend-only construction. Reject invalid/over-budget complete projections.
    pub fn new(
        root: TreeRootSnapshot,
        stamp: TreeViewStamp,
        layout_revision: TreeLayoutRevision,
        rows: Vec<TreeDisplayRow>,
    ) -> Result<Self, &'static str> {
        Self::new_with_root_actions(root.clone(), root, stamp, layout_revision, rows)
    }

    /// Construct a native maintenance publication with current root action identities.
    pub fn new_with_root_actions(
        root: TreeRootSnapshot,
        root_actions: TreeRootSnapshot,
        stamp: TreeViewStamp,
        layout_revision: TreeLayoutRevision,
        rows: Vec<TreeDisplayRow>,
    ) -> Result<Self, &'static str> {
        if stamp.session.0 == 0 || stamp.revision.0 == 0 || layout_revision.0 == 0 {
            return Err("tree projection requires live identities");
        }
        if root.pane != stamp.pane || root.navigation != stamp.navigation {
            return Err("tree projection belongs to a different root generation");
        }
        root.validate()?;
        validate_root_actions(&root, &root_actions)?;
        if rows.len() > Self::MAX_ROWS {
            return Err("tree projection exceeds its row budget");
        }
        let mut bytes = rows
            .capacity()
            .saturating_mul(size_of::<TreeDisplayRow>())
            .saturating_add(root_bytes(&root))
            .saturating_add(size_of::<TreeDisplayIndex>())
            .saturating_add(size_of::<Self>())
            .saturating_add(ARC_HEADER.saturating_mul(3));
        if !root.same_snapshot(&root_actions) {
            bytes = bytes.saturating_add(root_bytes(&root_actions));
        }
        let mut row_positions = HashMap::with_capacity(rows.len());
        let mut entry_ids = HashSet::with_capacity(rows.len());
        let mut root_rows = HashMap::with_capacity(root.entries.len().min(rows.len()));
        let mut parents: Vec<TreeRowId> = Vec::new();
        for (position, row) in rows.iter().enumerate() {
            if !root.same_snapshot(&root_actions)
                && row.depth == 0
                && row.original_root_entry.is_none()
            {
                return Err("current native root rows require exact action aliases");
            }
            bytes = bytes.saturating_add(row_bytes(row));
            if bytes > Self::MAX_BYTES {
                return Err("tree projection exceeds its payload budget");
            }
            if row.id.0 == 0
                || row_positions.insert(row.id, position).is_some()
                || !entry_ids.insert(row.entry.id)
            {
                return Err("tree projection has duplicate or invalid identities");
            }
            if row.depth > Self::MAX_DEPTH || row.depth > parents.len() {
                return Err("tree projection has invalid depth");
            }
            parents.truncate(row.depth);
            if row.parent.as_ref() != parents.last() {
                return Err("tree projection is not in ancestor order");
            }
            if let Some(parent) = row.parent
                && !rows[*row_positions.get(&parent).ok_or("tree parent is missing")?].expanded
            {
                return Err("collapsed tree parent cannot expose descendants");
            }
            if let Some(root_id) = row.original_root_entry
                && (row.parent.is_some() || root_rows.insert(root_id, row.id).is_some())
            {
                return Err("tree root aliases must identify distinct root rows");
            }
            if let Some(root_id) = row.original_root_entry {
                let entry = root_actions
                    .index
                    .by_id
                    .get(&root_id)
                    .and_then(|position| root_actions.entries.get(*position))
                    .ok_or("tree root alias is absent from its captured listing")?;
                if !same_entry_identity(entry, &row.entry) {
                    return Err("tree root alias refers to a different entry");
                }
            }
            if row.context_only && (row.selectable || !row.expanded || row.search_match) {
                return Err("tree context ancestors must be expanded, unmatched and unselectable");
            }
            if (row.expanded
                && !matches!(
                    row.disclosure,
                    TreeDisclosure::Expanded | TreeDisclosure::Loading | TreeDisclosure::Failed
                ))
                || (row.disclosure != TreeDisclosure::None && !row.navigable)
                || (row.entry.kind == EntryKind::DirectoryLink
                    && row.disclosure != TreeDisclosure::None)
                || (!row.expanded && row.disclosure == TreeDisclosure::Expanded)
            {
                return Err("tree disclosure contradicts its expanded state");
            }
            if row.entry.virtual_node.as_ref().is_some_and(|node| {
                !row.entry.path.as_os_str().is_empty() || node.kind != row.entry.kind
            }) {
                return Err("virtual tree entry has an invalid identity");
            }
            if row
                .entry
                .native_path()
                .is_some_and(|path| !path.is_absolute())
            {
                return Err("native tree display entry requires its full raw path");
            }
            if !row.entry.display_name.is_char_boundary(row.prefix_end)
                || row
                    .error
                    .as_ref()
                    .is_some_and(|error| error.capacity() > 4096)
            {
                return Err("tree display text exceeds its bounds");
            }
            parents.push(row.id);
        }
        validate_context(&rows)?;
        let search_rows: Arc<[usize]> = rows
            .iter()
            .enumerate()
            .filter_map(|(position, row)| row.search_match.then_some(position))
            .collect();
        let directory = Arc::new(DirectoryIndex::from_entry_refs(
            rows.iter().map(|row| &row.entry),
        ));
        bytes = bytes
            .saturating_add(map_bytes(&row_positions))
            .saturating_add(map_bytes(&root_rows))
            .saturating_add(size_of_val(search_rows.as_ref()))
            .saturating_add(ARC_HEADER)
            .saturating_add(directory_bytes(&directory));
        if bytes > Self::MAX_BYTES {
            return Err("tree projection exceeds its payload budget");
        }
        Ok(Self {
            root,
            root_actions,
            stamp,
            layout_revision,
            rows: rows.into(),
            index: Arc::new(TreeDisplayIndex {
                row_positions,
                search_rows,
                root_rows,
                directory,
            }),
        })
    }

    /// Backend comparison for gesture stability; metadata/disclosure changes alone are harmless.
    pub fn same_structure(&self, other: &Self) -> bool {
        self.root.same_snapshot(&other.root)
            && self.stamp.session == other.stamp.session
            && self.rows.len() == other.rows.len()
            && self
                .rows
                .iter()
                .zip(other.rows.iter())
                .all(|(left, right)| {
                    left.id == right.id
                        && left.entry.id == right.entry.id
                        && same_entry_identity(&left.entry, &right.entry)
                        && left.parent == right.parent
                        && left.depth == right.depth
                        && left.selectable == right.selectable
                        && left.navigable == right.navigable
                        && left.context_only == right.context_only
                        && left.original_root_entry == right.original_root_entry
                })
    }
}

fn validate_root_actions(
    root: &TreeRootSnapshot,
    actions: &TreeRootSnapshot,
) -> Result<(), &'static str> {
    if root.pane != actions.pane
        || root.navigation != actions.navigation
        || !root.within_payload_budget()
        || !actions.within_payload_budget()
    {
        return Err("tree action snapshot has a foreign generation or exceeds its budget");
    }
    actions.validate()?;
    if root.same_snapshot(actions) {
        return Ok(());
    }
    let mut parent = None;
    for entry in root.entries.iter().chain(actions.entries.iter()) {
        let path = entry
            .native_path()
            .ok_or("archive root actions must retain their captured snapshot")?;
        let actual_parent = path.parent().ok_or("native root action has no parent")?;
        if !path.is_absolute()
            || path.as_os_str().as_encoded_bytes().contains(&0)
            || path.file_name().is_none_or(|name| name != entry.name)
            || parent.is_some_and(|parent| parent != actual_parent)
        {
            return Err("native root action has invalid raw identity or parent");
        }
        parent = Some(actual_parent);
    }
    if actions.entries.iter().any(|entry| entry.id.0 == 0) {
        return Err("native root action alias must be nonzero");
    }
    Ok(())
}

fn row_bytes(row: &TreeDisplayRow) -> usize {
    entry_bytes(&row.entry).saturating_add(row.error.as_ref().map_or(0, String::capacity))
}

const ARC_HEADER: usize = 2 * size_of::<usize>();

fn entry_bytes(entry: &Entry) -> usize {
    let localized = entry
        .metadata
        .localized_modified
        .as_ref()
        .map_or(0, |value| {
            value.date.capacity().saturating_add(value.time.capacity())
        });
    let virtual_bytes = entry.virtual_node.as_ref().map_or(0, |node| {
        ARC_HEADER
            .saturating_add(node.raw_name.len())
            .saturating_add(node.display_name.capacity())
    });
    entry
        .path
        .capacity()
        .saturating_add(entry.name.capacity())
        .saturating_add(entry.display_name.capacity())
        .saturating_add(entry.archive_extensions.retained_bytes())
        .saturating_add(entry.collation.as_ref().map_or(0, |keys| {
            keys.retained_bytes()
                .saturating_add(size_of_val(keys.as_ref()))
                .saturating_add(ARC_HEADER.saturating_mul(2))
        }))
        .saturating_add(localized)
        .saturating_add(virtual_bytes)
}

fn root_bytes(root: &TreeRootSnapshot) -> usize {
    root.entries
        .iter()
        .fold(
            root.entries
                .len()
                .saturating_mul(size_of::<Entry>())
                .saturating_add(ARC_HEADER),
            |bytes, entry| bytes.saturating_add(entry_bytes(entry)),
        )
        .saturating_add(directory_bytes(&root.index))
}

fn directory_bytes(index: &DirectoryIndex) -> usize {
    size_of::<DirectoryIndex>()
        .saturating_add(index.column_samples.retained_bytes())
        .saturating_add(ARC_HEADER)
        .saturating_add(map_bytes(&index.by_id))
        .saturating_add(map_bytes(&index.by_path))
        .saturating_add(
            index
                .by_path
                .keys()
                .fold(0usize, |bytes, path| bytes.saturating_add(path.capacity())),
        )
        .saturating_add(
            index
                .directory_bits
                .capacity()
                .saturating_mul(size_of::<u64>()),
        )
        .saturating_add(
            index
                .file_sizes
                .capacity()
                .saturating_mul(size_of::<Option<u64>>()),
        )
}

fn map_bytes<K, V>(map: &HashMap<K, V>) -> usize {
    if map.capacity() == 0 {
        return 0;
    }
    // Capacity excludes spare buckets. Charge the next power of two plus control bytes.
    map.capacity()
        .saturating_add(1)
        .checked_next_power_of_two()
        .unwrap_or(usize::MAX)
        .saturating_mul(size_of::<(K, V)>().saturating_add(1))
        .saturating_add(16)
}

fn same_entry_identity(left: &Entry, right: &Entry) -> bool {
    left.kind == right.kind
        && match (&left.virtual_node, &right.virtual_node) {
            (None, None) => left.path == right.path,
            (Some(left), Some(right)) => {
                left.id == right.id
                    && left.parent == right.parent
                    && left.raw_name == right.raw_name
            }
            _ => false,
        }
}

fn validate_context(rows: &[TreeDisplayRow]) -> Result<(), &'static str> {
    let mut matches = [false; TreeProjection::MAX_DEPTH + 1];
    for row in rows.iter().rev() {
        if row.context_only && !matches[row.depth] {
            return Err("tree context ancestor has no visible matching descendant");
        }
        let has_match = !row.context_only || matches[row.depth];
        matches[row.depth] = false;
        if row.depth > 0 {
            matches[row.depth - 1] |= has_match;
        }
    }
    Ok(())
}

/// Shared selection, scoped to one exact display projection rather than native paths.
#[derive(Clone, Debug)]
pub struct TreeSelectionSnapshot {
    basis: Arc<TreeProjection>,
    bits: Arc<[u64]>,
    totals: ArchiveSelectionTotals,
}

impl TreeSelectionSnapshot {
    /// Backend-only; context rows and positions outside the projection cannot be selected.
    pub fn from_bits(basis: Arc<TreeProjection>, bits: Vec<u64>) -> Result<Self, &'static str> {
        let count = basis.rows.len();
        if bits.len() != count.div_ceil(64)
            || (!count.is_multiple_of(64)
                && bits.last().is_some_and(|word| *word >> (count % 64) != 0))
        {
            return Err("tree selection does not match its display basis");
        }
        let mut totals = ArchiveSelectionTotals::default();
        for (position, row) in basis.rows.iter().enumerate() {
            if bits[position / 64] & (1 << (position % 64)) != 0 {
                if !row.selectable || row.context_only {
                    return Err("tree context or unavailable row cannot be selected");
                }
                totals.change(row.entry.kind, row.entry.metadata.size, true);
            }
        }
        Ok(Self {
            basis,
            bits: bits.into(),
            totals,
        })
    }

    pub fn basis(&self) -> &Arc<TreeProjection> {
        &self.basis
    }
    /// Reconcile one session's selection by surviving visible occurrence identities on a worker.
    pub fn rebase(&self, basis: Arc<TreeProjection>) -> Result<Self, &'static str> {
        if !self.basis.root.same_snapshot(&basis.root)
            || self.basis.stamp.session != basis.stamp.session
            || basis.stamp.revision.0 < self.basis.stamp.revision.0
            || (basis.stamp.revision == self.basis.stamp.revision
                && !Arc::ptr_eq(&self.basis, &basis))
        {
            return Err("tree selection cannot rebase across foreign or stale views");
        }
        let mut bits = vec![0; basis.rows.len().div_ceil(64)];
        for (position, row) in basis.rows.iter().enumerate() {
            if row.selectable && !row.context_only && self.contains_row(row.id) {
                bits[position / 64] |= 1 << (position % 64);
            }
        }
        Self::from_bits(basis, bits)
    }
    pub fn bits(&self) -> &[u64] {
        &self.bits
    }
    pub fn len(&self) -> usize {
        self.totals.files() + self.totals.folders()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn file_count(&self) -> usize {
        self.totals.files()
    }
    pub fn folder_count(&self) -> usize {
        self.totals.folders()
    }
    pub fn file_bytes(&self) -> Option<u64> {
        self.totals.file_bytes()
    }
    pub fn contains_position(&self, position: usize) -> bool {
        position < self.basis.rows.len()
            && self
                .bits
                .get(position / 64)
                .is_some_and(|word| word & (1 << (position % 64)) != 0)
    }
    pub fn contains_row(&self, row: TreeRowId) -> bool {
        self.basis
            .index
            .row_positions
            .get(&row)
            .is_some_and(|position| self.contains_position(*position))
    }
}

#[derive(Clone, Debug)]
pub enum TreeSelectionAction {
    /// Remove at most 64 exact visible occurrences, idempotently.
    Remove(Arc<[TreeRowId]>),
    Toggle(TreeRowId),
    Range {
        anchor: TreeRowId,
        target: TreeRowId,
        additive: bool,
    },
    Ranges {
        ranges: SelectionRanges,
        mode: SelectionMode,
    },
    Bulk(SelectionOperation),
    Rebase,
}

#[derive(Clone, Debug)]
pub struct TreeSelectionRequest {
    pub view: Arc<TreeProjection>,
    pub revision: SelectionRevision,
    pub seed: Option<Arc<TreeSelectionSnapshot>>,
    pub seed_revision: SelectionRevision,
    pub action: TreeSelectionAction,
}
