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
    /// Inclusive positions resolved from raw paths in the captured immutable view.
    Range {
        anchor: PathBuf,
        target: PathBuf,
        additive: bool,
    },
    Rebase,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectionOrigin {
    pub navigation: NavigationId,
    pub location: PathBuf,
}

/// A comparison snapshot whose identity survives per-navigation revision resets.
/// The worker consumes each monotonic stamp once, before protecting an old source.
#[derive(Clone, Debug)]
pub struct AuthoritativeSelection {
    pub id: u64,
    pub origin: SelectionOrigin,
    pub snapshot: Arc<SelectionSnapshot>,
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
    /// Revision of an authoritative snapshot, including a two-panel comparison.
    /// A worker adopts a newer seed once, then preserves subsequent queued edits.
    pub seed_revision: SelectionRevision,
    pub authoritative: Option<Arc<AuthoritativeSelection>>,
    pub reset: bool,
    pub preserve: Option<SelectionOrigin>,
    pub restore: Option<SelectionOrigin>,
    pub action: SelectionAction,
    /// Native kind for a single toggle received before a complete index exists.
    pub toggle_directory: bool,
}

/// Bits refer to positions in `basis`, never to numeric EntryId values.
/// Paths remain authoritative when a refreshed listing recycles entry IDs.
#[derive(Clone, Debug, Default)]
pub struct SelectionSnapshot {
    basis: Arc<DirectoryIndex>,
    bits: Arc<[u64]>,
    selected_count: usize,
    selected_folders: usize,
    file_bytes: Option<u64>,
}

impl SelectionSnapshot {
    /// Build on a backend worker, validating the bitset resource/identity shape.
    pub fn from_bits(basis: Arc<DirectoryIndex>, bits: Vec<u64>) -> Result<Self, &'static str> {
        let count = basis.by_id.len();
        if basis.by_path.len() != count {
            return Err("selection directory index has mismatched identities");
        }
        if basis.directory_bits.len() != count.div_ceil(64) {
            return Err("selection directory kinds do not match index length");
        }
        if basis.file_sizes.len() != count {
            return Err("selection file sizes do not match index length");
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
        let selected_folders = bits
            .iter()
            .zip(&basis.directory_bits)
            .map(|(selected, folders)| (selected & folders).count_ones() as usize)
            .sum();
        let mut file_bytes = Some(0_u64);
        for (word_index, word) in bits.iter().copied().enumerate() {
            // Folder totals are reported separately. Folder links are skipped
            // here too; other non-regular entries still make file bytes unknown.
            let mut remaining = word & !basis.directory_bits[word_index];
            while remaining != 0 {
                let index = word_index * 64 + remaining.trailing_zeros() as usize;
                file_bytes = file_bytes
                    .zip(basis.file_sizes[index])
                    .and_then(|(total, size)| total.checked_add(size));
                remaining &= remaining - 1;
            }
        }
        Ok(Self {
            basis,
            bits: bits.into(),
            selected_count,
            selected_folders,
            file_bytes,
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
    pub fn file_count(&self) -> usize {
        self.selected_count - self.selected_folders
    }
    pub fn folder_count(&self) -> usize {
        self.selected_folders
    }
    /// Exact known sum of selected files, computed on the selection worker.
    /// Folders and folder links are excluded. None includes unknown lengths,
    /// other selected non-regular entries and totals exceeding u64.
    pub fn file_bytes(&self) -> Option<u64> {
        self.file_bytes
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

    #[test]
    fn selected_bytes_are_exact_only_for_known_regular_files() {
        let mut entries: Vec<_> = (0..4)
            .map(|id| Entry::new(EntryId(id), format!("file-{id}").into(), EntryKind::File))
            .collect();
        entries[0].metadata.size = Some(12);
        entries[1].metadata.size = Some(34);
        entries[2].metadata.size = Some(u64::MAX);
        let index = Arc::new(DirectoryIndex::from_entries(&entries));
        let selected = SelectionSnapshot::from_bits(index, vec![0b11]).unwrap();
        assert_eq!(selected.file_bytes(), Some(46));
        assert_eq!(selected.with_bits(vec![0]).unwrap().file_bytes(), Some(0));
        assert_eq!(selected.with_bits(vec![0b1001]).unwrap().file_bytes(), None);
        assert_eq!(selected.with_bits(vec![0b0101]).unwrap().file_bytes(), None);
        entries[1].kind = EntryKind::Directory;
        let index = Arc::new(DirectoryIndex::from_entries(&entries));
        assert_eq!(
            SelectionSnapshot::from_bits(index, vec![3])
                .unwrap()
                .file_bytes(),
            Some(12)
        );
        entries[1].kind = EntryKind::DirectoryLink;
        let index = Arc::new(DirectoryIndex::from_entries(&entries));
        assert_eq!(
            SelectionSnapshot::from_bits(index, vec![3])
                .unwrap()
                .file_bytes(),
            Some(12)
        );
        entries[1].kind = EntryKind::Symlink;
        let index = Arc::new(DirectoryIndex::from_entries(&entries));
        assert_eq!(
            SelectionSnapshot::from_bits(index, vec![3])
                .unwrap()
                .file_bytes(),
            None
        );
    }

    #[test]
    fn selected_counts_distinguish_folders_links_and_other_entries() {
        let entries: Vec<_> = [
            EntryKind::Directory,
            EntryKind::DirectoryLink,
            EntryKind::File,
            EntryKind::Symlink,
            EntryKind::Other,
        ]
        .into_iter()
        .enumerate()
        .map(|(row, kind)| Entry::new(EntryId(row as u64), format!("row-{row}").into(), kind))
        .collect();
        let index = Arc::new(DirectoryIndex::from_entries(&entries));
        let all = SelectionSnapshot::from_bits(index, vec![0b11111]).unwrap();
        assert_eq!((all.file_count(), all.folder_count()), (3, 2));
        let links = all.with_bits(vec![0b01010]).unwrap();
        assert_eq!((links.file_count(), links.folder_count()), (1, 1));
        let mut incomplete = DirectoryIndex::from_entries(&entries);
        incomplete.directory_bits.clear();
        assert!(SelectionSnapshot::from_bits(Arc::new(incomplete), vec![1]).is_err());
    }
}
