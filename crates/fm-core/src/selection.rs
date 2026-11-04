//! Immutable, shareable selection data prepared by backend workers.
use crate::{DirectoryIndex, NavigationId, PaneId};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionOperation {
    All,
    Clear,
    Invert,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct SelectionRevision(pub u64);

#[derive(Clone, Debug)]
pub enum SelectionAction {
    Bulk(SelectionOperation),
    Toggle(PathBuf),
    Rebase,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectionOrigin {
    pub navigation: NavigationId,
    pub location: PathBuf,
}

/// Cheap UI-to-backend message; the index and previous selection are shared.
#[derive(Clone, Debug)]
pub struct SelectionRequest {
    pub pane: PaneId,
    pub navigation: NavigationId,
    pub revision: SelectionRevision,
    /// Stable across same-pane refresh, distinct for a new tab/location.
    pub context: NavigationId,
    pub location: PathBuf,
    pub view: Option<Arc<DirectoryIndex>>,
    pub seed: Arc<SelectionSnapshot>,
    pub reset: bool,
    pub preserve: Option<SelectionOrigin>,
    pub restore: Option<SelectionOrigin>,
    pub action: SelectionAction,
}

/// Bits refer to positions in `basis`, never to numeric EntryId values.
/// Paths remain authoritative when a refreshed listing recycles entry IDs.
#[derive(Clone, Debug, Default)]
pub struct SelectionSnapshot {
    basis: Arc<DirectoryIndex>,
    bits: Arc<[u64]>,
    selected_count: usize,
}

impl SelectionSnapshot {
    /// Build on a backend worker, validating the bitset resource/identity shape.
    pub fn from_bits(basis: Arc<DirectoryIndex>, bits: Vec<u64>) -> Result<Self, &'static str> {
        let count = basis.by_id.len();
        if basis.by_path.len() != count {
            return Err("selection directory index has mismatched identities");
        }
        let mut seen = vec![false; count];
        for position in basis.by_id.values() {
            let Some(used) = seen.get_mut(*position) else {
                return Err("selection directory index position is out of range");
            };
            if std::mem::replace(used, true) {
                return Err("selection directory index positions are duplicated");
            }
        }
        seen.fill(false);
        for id in basis.by_path.values() {
            let Some(position) = basis.by_id.get(id) else {
                return Err("selection directory path references a missing identity");
            };
            if std::mem::replace(&mut seen[*position], true) {
                return Err("selection directory paths reference duplicate identities");
            }
        }
        Self::checked_bits(basis, bits)
    }

    /// Replace bits on the already validated immutable basis, on a worker.
    pub fn with_bits(&self, bits: Vec<u64>) -> Result<Self, &'static str> {
        Self::checked_bits(self.basis.clone(), bits)
    }

    fn checked_bits(basis: Arc<DirectoryIndex>, bits: Vec<u64>) -> Result<Self, &'static str> {
        let count = basis.by_id.len();
        if bits.len() != count.div_ceil(64) {
            return Err("selection bitset does not match its directory index");
        }
        if !count.is_multiple_of(64) && bits.last().is_some_and(|last| *last >> (count % 64) != 0) {
            return Err("selection contains bits beyond the directory index");
        }
        let selected_count = bits.iter().map(|word| word.count_ones() as usize).sum();
        Ok(Self {
            basis,
            bits: bits.into(),
            selected_count,
        })
    }
    pub fn contains_path(&self, path: &Path) -> bool {
        self.basis
            .by_path
            .get(path)
            .and_then(|id| self.basis.by_id.get(id))
            .and_then(|index| {
                self.bits
                    .get(index / 64)
                    .map(|word| word & (1 << (index % 64)) != 0)
            })
            .unwrap_or(false)
    }
    pub fn len(&self) -> usize {
        self.selected_count
    }
    pub fn is_empty(&self) -> bool {
        self.selected_count == 0
    }
    pub fn basis(&self) -> &Arc<DirectoryIndex> {
        &self.basis
    }
    pub fn bits(&self) -> &[u64] {
        &self.bits
    }

    /// Enumeration may be linear in the whole listing; use only on a worker.
    pub fn selected_paths(&self) -> impl Iterator<Item = &PathBuf> {
        self.basis
            .by_path
            .keys()
            .filter(move |path| self.contains_path(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Entry, EntryId, EntryKind};

    #[test]
    fn bits_use_index_positions_and_preserve_exact_unicode_paths() {
        let entries = vec![
            Entry::new(EntryId(u64::MAX), "文档/e\u{301}".into(), EntryKind::File),
            Entry::new(EntryId(0), "文档/é".into(), EntryKind::Directory),
            Entry::new(EntryId(99), "👩🏽‍💻/a".into(), EntryKind::File),
        ];
        let index = Arc::new(DirectoryIndex::from_entries(&entries));
        let snapshot = SelectionSnapshot::from_bits(index.clone(), vec![0b101]).unwrap();
        assert_eq!(snapshot.len(), 2);
        assert!(snapshot.contains_path(&entries[0].path));
        assert!(!snapshot.contains_path(&entries[1].path));
        assert!(snapshot.contains_path(&entries[2].path));
        assert!(!snapshot.contains_path(Path::new("..")));
        assert!(!snapshot.contains_path(Path::new("missing")));
        assert!(Arc::ptr_eq(snapshot.basis(), &index));
        let cloned = snapshot.clone();
        assert!(Arc::ptr_eq(&snapshot.bits, &cloned.bits));
    }

    #[test]
    fn invalid_or_padded_bits_cannot_create_selected_phantom_rows() {
        let entries: Vec<_> = (0..65)
            .map(|id| Entry::new(EntryId(id), format!("row-{id}").into(), EntryKind::File))
            .collect();
        let index = Arc::new(DirectoryIndex::from_entries(&entries));
        assert!(SelectionSnapshot::from_bits(index.clone(), vec![0]).is_err());
        assert!(SelectionSnapshot::from_bits(index.clone(), vec![0, 2]).is_err());
        let snapshot = SelectionSnapshot::from_bits(index, vec![u64::MAX, 1]).unwrap();
        assert_eq!(snapshot.len(), 65);
        assert_eq!(snapshot.selected_paths().count(), 65);
    }

    #[test]
    fn forged_index_positions_and_path_identities_are_rejected() {
        let entries = vec![
            Entry::new(EntryId(1), "a".into(), EntryKind::File),
            Entry::new(EntryId(2), "b".into(), EntryKind::File),
        ];
        for mutation in 0..4 {
            let mut index = DirectoryIndex::from_entries(&entries);
            match mutation {
                0 => {
                    index.by_id.insert(EntryId(1), usize::MAX);
                }
                1 => {
                    index.by_id.insert(EntryId(2), 0);
                }
                2 => {
                    index.by_path.insert("b".into(), EntryId(99));
                }
                _ => {
                    index.by_path.insert("b".into(), EntryId(1));
                }
            }
            assert!(SelectionSnapshot::from_bits(Arc::new(index), vec![3]).is_err());
        }
    }
}
