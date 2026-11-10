use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    path::PathBuf,
    sync::Arc,
    time::SystemTime,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EntryId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Directory,
    /// A known directory symlink/junction. Browse explicitly; never recurse automatically.
    DirectoryLink,
    File,
    Symlink,
    Other,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EntryMetadata {
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
    /// Native locale/timezone formatting prepared on a backend worker.
    pub localized_modified: Option<crate::LocalizedDateTime>,
    /// Filesystem-reported compression, independent of file extension or size.
    pub compression: CompressionStatus,
    /// Raw Windows file attributes, present only when the native provider supplies them.
    pub native_attributes: Option<u32>,
    /// POSIX file type and permission bits, including setuid/setgid/sticky bits.
    pub unix_mode: Option<u32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CompressionStatus {
    #[default]
    Unknown,
    Uncompressed,
    Compressed,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub id: EntryId,
    pub name: OsString,
    pub path: PathBuf,
    /// Virtual identities never use `path`; it is empty for these entries.
    /// Native services must require `native_path()` rather than interpreting display names.
    pub virtual_node: Option<crate::ArchiveNode>,
    pub network_node: Option<crate::NetworkNode>,
    /// Backend filename hint; usable only while a provider advertises this format.
    pub archive_association: Option<&'static crate::ArchiveAssociation>,
    /// Literal custom suffix hints prepared during backend enumeration.
    pub archive_extensions: crate::ArchiveExtensionHints,
    pub kind: EntryKind,
    pub metadata: EntryMetadata,
    pub display_name: String,
    /// Session-only keys prepared once on a worker and shared by captured views.
    pub(crate) collation: Option<Arc<crate::sorting::EntryCollationKeys>>,
}

/// Built by a background worker so UI identity lookups never scan a full listing.
#[derive(Clone, Debug, Default)]
pub struct DirectoryIndex {
    pub by_id: HashMap<EntryId, usize>,
    pub by_path: HashMap<PathBuf, EntryId>,
    /// Includes directory links; prepared with the index on a backend worker.
    pub directory_bits: Vec<u64>,
    /// Logical regular-file lengths prepared with the index on a backend worker.
    /// Links, directories, special entries and unavailable metadata remain unknown.
    pub file_sizes: Vec<Option<u64>>,
}

impl DirectoryIndex {
    pub fn from_entries(entries: &[Entry]) -> Self {
        Self::from_entry_refs(entries.iter())
    }

    pub(crate) fn from_entry_refs<'a>(entries: impl ExactSizeIterator<Item = &'a Entry>) -> Self {
        let count = entries.len();
        let mut index = Self {
            by_id: HashMap::with_capacity(count),
            by_path: HashMap::with_capacity(count),
            directory_bits: vec![0; count.div_ceil(64)],
            file_sizes: Vec::with_capacity(count),
        };
        for (row, entry) in entries.enumerate() {
            index.by_id.insert(entry.id, row);
            if let Some(path) = entry.native_path() {
                index.by_path.insert(path.to_owned(), entry.id);
            }
            if matches!(entry.kind, EntryKind::Directory | EntryKind::DirectoryLink) {
                index.directory_bits[row / 64] |= 1 << (row % 64);
            }
            index.file_sizes.push(
                (entry.kind == EntryKind::File)
                    .then_some(entry.metadata.size)
                    .flatten(),
            );
        }
        index
    }
    pub fn is_directory(&self, id: EntryId) -> bool {
        self.by_id
            .get(&id)
            .and_then(|row| {
                self.directory_bits
                    .get(row / 64)
                    .map(|bits| bits & (1 << (row % 64)) != 0)
            })
            .unwrap_or(false)
    }
}

impl Entry {
    pub fn new(id: EntryId, path: PathBuf, kind: EntryKind) -> Self {
        let name = path.file_name().unwrap_or(path.as_os_str()).to_os_string();
        let display_name = name.to_string_lossy().into_owned();
        let archive_association = (kind == EntryKind::File)
            .then(|| crate::archive_association(&path))
            .flatten();
        let archive_extensions = if kind == EntryKind::File {
            crate::ArchiveExtensionHints::from_path(&path)
        } else {
            crate::ArchiveExtensionHints::default()
        };
        Self {
            id,
            name,
            path,
            virtual_node: None,
            network_node: None,
            archive_association,
            archive_extensions,
            kind,
            metadata: EntryMetadata::default(),
            display_name,
            collation: None,
        }
    }
    pub fn from_archive(node: crate::ArchiveNode) -> Self {
        let mut entry = Self::new(EntryId(node.id.entry), PathBuf::new(), node.kind);
        entry.name = OsString::from(&node.display_name);
        entry.display_name = node.display_name.clone();
        entry.metadata.size = node.size;
        if let Some(crate::ArchiveTimestamp::Utc {
            seconds,
            nanoseconds,
        }) = node.modified
            && nanoseconds < 1_000_000_000
        {
            entry.metadata.modified = if seconds >= 0 {
                std::time::UNIX_EPOCH
                    .checked_add(std::time::Duration::new(seconds as u64, nanoseconds))
            } else {
                std::time::UNIX_EPOCH
                    .checked_sub(std::time::Duration::from_secs(seconds.unsigned_abs()))
                    .and_then(|time| {
                        time.checked_add(std::time::Duration::from_nanos(u64::from(nanoseconds)))
                    })
            };
        }
        entry.virtual_node = Some(node);
        entry
    }
    pub fn native_path(&self) -> Option<&std::path::Path> {
        (self.virtual_node.is_none() && self.network_node.is_none()).then_some(self.path.as_path())
    }

    /// Borrow the original filename extension for presentation sorting. Archive raw names
    /// are provider basenames, never host paths; opaque encodings have no Unicode extension.
    pub(crate) fn filename_extension(&self) -> Option<&OsStr> {
        if let Some(node) = &self.virtual_node {
            let name = std::str::from_utf8(&node.raw_name).ok()?;
            if name == ".." {
                return None;
            }
            let (stem, extension) = name.rsplit_once('.')?;
            return (!stem.is_empty()).then_some(OsStr::new(extension));
        }
        self.native_path()?.extension()
    }

    pub fn from_network(id: EntryId, node: crate::NetworkNode) -> Self {
        let mut entry = Self::new(id, PathBuf::new(), EntryKind::Directory);
        entry.name = OsString::from(&node.display);
        entry.display_name = node.display.clone();
        entry.network_node = Some(node);
        entry
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sort_entries;

    fn entry(id: u64, name: &str, kind: EntryKind, size: Option<u64>) -> Entry {
        let mut result = Entry::new(EntryId(id), PathBuf::from(name), kind);
        result.metadata.size = size;
        result
    }

    #[test]
    fn directory_links_sort_with_folders_without_becoming_recursive_directories() {
        let mut entries = vec![
            entry(1, "a-file", EntryKind::File, None),
            entry(2, "z-junction", EntryKind::DirectoryLink, None),
            entry(3, "b-folder", EntryKind::Directory, None),
        ];
        sort_entries(&mut entries, SortSpec::default());
        assert_eq!(
            entries.iter().map(|entry| entry.id.0).collect::<Vec<_>>(),
            [3, 2, 1]
        );
        assert_ne!(entries[1].kind, EntryKind::Directory);
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
    fn names_without_native_keys_have_deterministic_raw_order() {
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
