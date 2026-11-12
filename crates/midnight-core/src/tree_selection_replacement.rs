//! Bounded selection intent without source projection or provider ownership.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Occurrence {
    id: TreeRowId,
    parent: Option<TreeRowId>,
    kind: EntryKind,
    selectable: bool,
    context_only: bool,
    original_root_entry: Option<EntryId>,
}
impl From<&TreeDisplayRow> for Occurrence {
    fn from(row: &TreeDisplayRow) -> Self {
        Self {
            id: row.id,
            parent: row.parent,
            kind: row.entry.kind,
            selectable: row.selectable,
            context_only: row.context_only,
            original_root_entry: row.original_root_entry,
        }
    }
}

/// Immutable marks and occurrence identities; never native operation authority.
/// Fields remain private so command admission can validate bounds without walking
/// the rows. Both buffers contain only Copy values, with no paths, reports, roots,
/// projections or provider owners to destroy when a UI preview is retired.
#[derive(Debug)]
pub struct TreeSelectionReplacement {
    stamp: TreeViewStamp,
    layout: TreeLayoutRevision,
    rows: Box<[Occurrence]>,
    bits: Box<[u64]>,
}
impl TreeSelectionReplacement {
    pub const MAX_BYTES: usize = 512 * 1024;

    /// Build on a backend worker, copying at most MAX_ROWS numeric occurrences.
    /// Reject excessive shape before allocating either bounded destination buffer.
    pub fn from_snapshot(selection: &TreeSelectionSnapshot) -> Result<Self, &'static str> {
        let view = selection.basis();
        let count = view.rows.len();
        if count > TreeProjection::MAX_ROWS
            || selection.bits().len() != count.div_ceil(64)
            || Self::retained_bytes(count, selection.bits().len()) > Self::MAX_BYTES
        {
            return Err("tree selection replacement exceeds its payload budget");
        }
        Ok(Self {
            stamp: view.stamp,
            layout: view.layout_revision,
            rows: view.rows.iter().map(Occurrence::from).collect(),
            bits: selection.bits().into(),
        })
    }

    pub fn stamp(&self) -> TreeViewStamp {
        self.stamp
    }
    pub fn layout_revision(&self) -> TreeLayoutRevision {
        self.layout
    }
    pub fn bits(&self) -> &[u64] {
        &self.bits
    }

    /// Constant-time lookup fenced by both position and original occurrence ID.
    pub fn contains_position(&self, position: usize, row: TreeRowId) -> bool {
        self.rows.get(position).is_some_and(|item| item.id == row)
            && self
                .bits
                .get(position / 64)
                .is_some_and(|word| word & (1 << (position % 64)) != 0)
    }

    /// Allocation-free command ingress validation; private boxed slices have no spare capacity.
    pub fn within_payload_budget(&self) -> bool {
        self.rows.len() <= TreeProjection::MAX_ROWS
            && self.bits.len() == self.rows.len().div_ceil(64)
            && Self::retained_bytes(self.rows.len(), self.bits.len()) <= Self::MAX_BYTES
    }

    /// Constant-time shape/scope check before a bounded mailbox retains this intent.
    /// The model worker must additionally call matches_view before applying marks.
    pub fn admits_view(&self, view: &TreeProjection) -> bool {
        self.within_payload_budget()
            && self.stamp.pane == view.stamp.pane
            && self.stamp.navigation == view.stamp.navigation
            && self.stamp.session == view.stamp.session
            && self.stamp.revision.0 <= view.stamp.revision.0
            && self.layout == view.layout_revision
            && self.rows.len() == view.rows.len()
    }

    /// Constant-time UI correlation for a result whose request owns the exact displayed Arc.
    pub fn matches_origin(&self, view: &TreeProjection) -> bool {
        self.stamp == view.stamp && self.admits_view(view)
    }

    /// Worker-only validation of every occurrence, allowing metadata/disclosure updates only.
    pub fn matches_view(&self, view: &TreeProjection) -> bool {
        self.admits_view(view)
            && self
                .rows
                .iter()
                .zip(view.rows.iter())
                .all(|(before, after)| *before == Occurrence::from(after))
    }

    fn retained_bytes(rows: usize, words: usize) -> usize {
        size_of::<Self>()
            .saturating_add(rows.saturating_mul(size_of::<Occurrence>()))
            .saturating_add(words.saturating_mul(size_of::<u64>()))
    }
}
