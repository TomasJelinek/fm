//! Incremental selected-folder status totals, owned by backend workers.
use midnight_core::{
    Entry, EntryKind, FolderSize, SelectedFolderSizes as Summary, SelectionSnapshot,
};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Copy, PartialEq, Eq)]
struct Contribution {
    bytes: u64,
    complete: bool,
}

#[derive(Default)]
pub(crate) struct Aggregate {
    folders: usize,
    eligible: HashSet<PathBuf>,
    values: HashMap<PathBuf, Contribution>,
    complete: usize,
    partial: usize,
    // The mathematical maximum is usize::MAX contributions of u64::MAX bytes,
    // which fits u128 on supported 32/64-bit targets. The public u64 conversion
    // remains checked and can recover when a large contribution is replaced.
    complete_bytes: u128,
    partial_bytes: u128,
}

impl Aggregate {
    pub(crate) fn new(
        selection: Arc<SelectionSnapshot>,
        entries: &[Entry],
        sizes: &HashMap<PathBuf, FolderSize>,
    ) -> Self {
        let mut aggregate = Self {
            folders: selection.folder_count(),
            eligible: entries
                .iter()
                .filter(|entry| {
                    entry.kind == EntryKind::Directory && selection.contains_path(&entry.path)
                })
                .map(|entry| entry.path.clone())
                .collect(),
            ..Self::default()
        };
        for (path, size) in sizes {
            aggregate.update(path, Some(*size));
        }
        aggregate
    }

    pub(crate) fn update(&mut self, path: &Path, size: Option<FolderSize>) -> bool {
        if !self.eligible.contains(path) {
            return false;
        }
        let value = size.map(|size| Contribution {
            bytes: size.bytes,
            complete: size.finished && size.errors == 0,
        });
        if self.values.get(path).copied() == value {
            return false;
        }
        if let Some(previous) = self.values.remove(path) {
            if previous.complete {
                self.complete -= 1;
                self.complete_bytes -= u128::from(previous.bytes);
            } else {
                self.partial -= 1;
                self.partial_bytes -= u128::from(previous.bytes);
            }
        }
        if let Some(value) = value {
            if value.complete {
                self.complete += 1;
                self.complete_bytes += u128::from(value.bytes);
            } else {
                self.partial += 1;
                self.partial_bytes += u128::from(value.bytes);
            }
            self.values.insert(path.to_owned(), value);
        }
        true
    }

    pub(crate) fn summary(&self) -> Summary {
        Summary {
            folders: self.folders,
            complete: self.complete,
            complete_bytes: u64::try_from(self.complete_bytes).ok(),
            partial: self.partial,
            partial_bytes: u64::try_from(self.partial_bytes).ok(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use midnight_core::{DirectoryIndex, EntryId, EntryKind};

    fn selection(
        rows: &[(&str, EntryKind)],
        selected: u64,
    ) -> (Arc<SelectionSnapshot>, Vec<Entry>) {
        let entries: Vec<_> = rows
            .iter()
            .enumerate()
            .map(|(id, (name, kind))| {
                Entry::new(EntryId(id as u64), Path::new("active").join(name), *kind)
            })
            .collect();
        let selected = Arc::new(
            SelectionSnapshot::from_bits(
                Arc::new(DirectoryIndex::from_entries(&entries)),
                vec![selected],
            )
            .unwrap(),
        );
        (selected, entries)
    }

    fn size(bytes: u64, finished: bool, errors: u64) -> FolderSize {
        FolderSize {
            bytes,
            finished,
            errors,
            ..Default::default()
        }
    }

    fn cache(values: &[(&str, FolderSize)]) -> HashMap<PathBuf, FolderSize> {
        values
            .iter()
            .map(|(name, size)| (Path::new("active").join(name), *size))
            .collect()
    }

    #[test]
    fn exact_totals_exclude_partial_unknown_file_and_unselected_contributions() {
        let (selected, entries) = selection(
            &[
                ("known", EntryKind::Directory),
                ("partial", EntryKind::Directory),
                ("unknown", EntryKind::Directory),
                ("link", EntryKind::DirectoryLink),
                ("file", EntryKind::File),
                ("unselected", EntryKind::Directory),
            ],
            0b011111,
        );
        let totals = Aggregate::new(
            selected,
            &entries,
            &cache(&[
                ("known", size(21, true, 0)),
                ("partial", size(8, false, 0)),
                ("link", size(555, true, 0)),
                ("file", size(999, true, 0)),
                ("unselected", size(777, true, 0)),
            ]),
        );
        assert_eq!(
            totals.summary(),
            Summary {
                folders: 4,
                complete: 1,
                complete_bytes: Some(21),
                partial: 1,
                partial_bytes: Some(8),
            }
        );
    }

    #[test]
    fn replaced_progress_moves_between_partial_and_complete_without_double_counting() {
        let (selected, entries) = selection(&[("folder", EntryKind::Directory)], 1);
        let mut totals = Aggregate::new(selected, &entries, &HashMap::new());
        let path = Path::new("active/folder");
        assert_eq!(
            totals.summary(),
            Summary {
                folders: 1,
                complete_bytes: Some(0),
                partial_bytes: Some(0),
                ..Default::default()
            }
        );
        assert!(totals.update(path, Some(size(5, false, 0))));
        assert!(totals.update(path, Some(size(11, false, 0))));
        assert_eq!(totals.summary().partial_bytes, Some(11));
        assert_eq!(totals.summary().complete_bytes, Some(0));
        assert!(totals.update(path, Some(size(13, true, 0))));
        assert_eq!(
            totals.summary(),
            Summary {
                folders: 1,
                complete: 1,
                complete_bytes: Some(13),
                partial: 0,
                partial_bytes: Some(0),
            }
        );
        assert!(!totals.update(path, Some(size(13, true, 0))));
        assert!(totals.update(path, None));
        assert_eq!(totals.summary().complete, 0);
        assert_eq!(totals.summary().complete_bytes, Some(0));
    }

    #[test]
    fn completed_errors_and_cancelled_values_are_partial_even_when_zero_bytes() {
        let (selected, entries) = selection(
            &[
                ("denied", EntryKind::Directory),
                ("cancelled", EntryKind::Directory),
                ("empty", EntryKind::Directory),
            ],
            7,
        );
        let totals = Aggregate::new(
            selected,
            &entries,
            &cache(&[
                ("denied", size(8, true, 1)),
                ("cancelled", size(0, false, 0)),
                ("empty", size(0, true, 0)),
            ]),
        );
        assert_eq!(
            totals.summary(),
            Summary {
                folders: 3,
                complete: 1,
                complete_bytes: Some(0),
                partial: 2,
                partial_bytes: Some(8),
            }
        );
    }

    #[test]
    fn native_unicode_membership_does_not_merge_normalization_or_include_files() {
        let (selected, entries) = selection(
            &[
                ("é", EntryKind::Directory),
                ("e\u{301}", EntryKind::Directory),
                ("文档.txt", EntryKind::File),
            ],
            5,
        );
        let mut totals = Aggregate::new(selected, &entries, &HashMap::new());
        assert!(!totals.update(Path::new("active/e\u{301}"), Some(size(999, true, 0))));
        assert!(!totals.update(Path::new("active/文档.txt"), Some(size(999, true, 0))));
        assert!(totals.update(Path::new("active/é"), Some(size(17, true, 0))));
        assert_eq!(totals.summary().complete_bytes, Some(17));
        assert_eq!(totals.summary().complete, 1);
    }

    #[test]
    fn overflow_is_explicit_and_recovers_after_a_contribution_is_replaced() {
        let (selected, entries) = selection(
            &[
                ("huge", EntryKind::Directory),
                ("other", EntryKind::Directory),
            ],
            3,
        );
        let mut totals = Aggregate::new(
            selected,
            &entries,
            &cache(&[
                ("huge", size(u64::MAX, true, 0)),
                ("other", size(1, true, 0)),
            ]),
        );
        assert_eq!(totals.summary().complete, 2);
        assert_eq!(totals.summary().complete_bytes, None);
        assert!(totals.update(Path::new("active/other"), Some(size(0, true, 0))));
        assert_eq!(totals.summary().complete_bytes, Some(u64::MAX));
        assert!(totals.update(Path::new("active/huge"), Some(size(u64::MAX, false, 0))));
        assert!(totals.update(Path::new("active/other"), Some(size(1, false, 0))));
        assert_eq!(totals.summary().complete_bytes, Some(0));
        assert_eq!(totals.summary().partial_bytes, None);
    }
}
