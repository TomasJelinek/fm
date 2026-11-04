use std::{collections::HashMap, ffi::OsString, path::PathBuf, time::SystemTime};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EntryId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Directory,
    File,
    Symlink,
    Other,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EntryMetadata {
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub id: EntryId,
    pub name: OsString,
    pub path: PathBuf,
    pub kind: EntryKind,
    pub metadata: EntryMetadata,
    pub display_name: String,
    pub sort_name: String,
}

/// Built by a background worker so UI identity lookups never scan a full listing.
#[derive(Clone, Debug, Default)]
pub struct DirectoryIndex {
    pub by_id: HashMap<EntryId, usize>,
    pub by_path: HashMap<PathBuf, EntryId>,
}

impl DirectoryIndex {
    pub fn from_entries(entries: &[Entry]) -> Self {
        Self {
            by_id: entries
                .iter()
                .enumerate()
                .map(|(index, entry)| (entry.id, index))
                .collect(),
            by_path: entries
                .iter()
                .map(|entry| (entry.path.clone(), entry.id))
                .collect(),
        }
    }
}

impl Entry {
    pub fn new(id: EntryId, path: PathBuf, kind: EntryKind) -> Self {
        let name = path.file_name().unwrap_or(path.as_os_str()).to_os_string();
        let display_name = name.to_string_lossy().into_owned();
        let sort_name = display_name.to_lowercase();
        Self {
            id,
            name,
            path,
            kind,
            metadata: EntryMetadata::default(),
            display_name,
            sort_name,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortColumn {
    #[default]
    Name,
    Size,
    Modified,
    Type,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SortSpec {
    pub column: SortColumn,
    pub descending: bool,
}

pub fn sort_entries(entries: &mut [Entry], sort: SortSpec) {
    use std::cmp::Ordering;

    fn optional<T: Ord>(left: Option<T>, right: Option<T>, descending: bool) -> Ordering {
        match (left, right) {
            (Some(left), Some(right)) => {
                let order = left.cmp(&right);
                if descending { order.reverse() } else { order }
            }
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        }
    }

    entries.sort_unstable_by(|left, right| {
        let directory_order =
            (right.kind == EntryKind::Directory).cmp(&(left.kind == EntryKind::Directory));
        if directory_order != Ordering::Equal {
            return directory_order;
        }
        let primary = match sort.column {
            SortColumn::Name => {
                let order = left.sort_name.cmp(&right.sort_name);
                if sort.descending {
                    order.reverse()
                } else {
                    order
                }
            }
            SortColumn::Size => optional(left.metadata.size, right.metadata.size, sort.descending),
            SortColumn::Modified => optional(
                left.metadata.modified,
                right.metadata.modified,
                sort.descending,
            ),
            SortColumn::Type => optional(
                left.path.extension(),
                right.path.extension(),
                sort.descending,
            ),
        };
        primary
            .then_with(|| left.sort_name.cmp(&right.sort_name))
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.id.0.cmp(&right.id.0))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u64, name: &str, kind: EntryKind, size: Option<u64>) -> Entry {
        let mut result = Entry::new(EntryId(id), PathBuf::from(name), kind);
        result.metadata.size = size;
        result
    }

    #[test]
    fn folders_stay_first_when_sorting_descending() {
        let mut entries = vec![
            entry(1, "z.txt", EntryKind::File, Some(4)),
            entry(2, "folder", EntryKind::Directory, None),
            entry(3, "a.txt", EntryKind::File, Some(2)),
        ];
        sort_entries(
            &mut entries,
            SortSpec {
                column: SortColumn::Name,
                descending: true,
            },
        );
        assert_eq!(
            entries.iter().map(|entry| entry.id.0).collect::<Vec<_>>(),
            [2, 1, 3]
        );
    }

    #[test]
    fn unknown_sizes_stay_last_in_both_directions() {
        for descending in [false, true] {
            let mut entries = vec![
                entry(1, "unknown", EntryKind::File, None),
                entry(2, "large", EntryKind::File, Some(50)),
                entry(3, "small", EntryKind::File, Some(2)),
            ];
            sort_entries(
                &mut entries,
                SortSpec {
                    column: SortColumn::Size,
                    descending,
                },
            );
            let want = if descending {
                vec![2, 3, 1]
            } else {
                vec![3, 2, 1]
            };
            assert_eq!(
                entries.iter().map(|entry| entry.id.0).collect::<Vec<_>>(),
                want
            );
        }
    }

    #[test]
    fn equal_casefolded_names_have_deterministic_raw_name_order() {
        let mut entries = vec![
            entry(2, "alpha", EntryKind::File, None),
            entry(1, "Alpha", EntryKind::File, None),
        ];
        sort_entries(&mut entries, SortSpec::default());
        assert_eq!(
            entries.iter().map(|entry| entry.id.0).collect::<Vec<_>>(),
            [1, 2]
        );
    }

    #[test]
    fn index_resolves_identity_and_raw_path_after_sorting() {
        let entries = vec![
            entry(9, "first", EntryKind::File, None),
            entry(2, "second", EntryKind::File, None),
        ];
        let index = DirectoryIndex::from_entries(&entries);
        assert_eq!(index.by_id.get(&EntryId(2)), Some(&1));
        assert_eq!(
            index.by_path.get(&PathBuf::from("first")),
            Some(&EntryId(9))
        );
    }
}
