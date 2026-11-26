#![forbid(unsafe_code)]
//! Portable, blocking local directory and capacity services shared by native adapters.
pub mod comparator_rules;
pub mod content_pattern;
pub mod editor_rules;
pub mod filename_colors;
pub mod filter;
pub mod filter_expression;
pub mod fonts;
pub mod icon_scale;
pub mod icon_snapshot;
pub mod license_notices;
pub mod metadata;
pub mod settings;
pub mod transfer;
pub mod transfer_io;
pub mod transfer_modified_time;
pub mod transfer_namespace;

use midnight_core::{
    Cancellation, DirectoryService, Entry, EntryId, EntryKind, EntryMetadata, FsError,
};
use std::{
    fs, io,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

/// Resolve the path field on the picker worker. A directory typed in a file field
/// opens that directory. An unfinished folder path opens its nearest existing parent.
pub fn picker_start(
    request: &midnight_core::PathPickerRequest,
    cancel: &Cancellation,
) -> midnight_core::PathPickerRequest {
    let mut start = request.clone();
    if let Some(folder) = &request.initial_directory {
        if let Some(name) = &request.initial_name {
            let candidate = folder.join(name);
            if !cancel.is_cancelled() && candidate.is_dir() {
                start.initial_directory = Some(candidate);
                start.initial_name = None;
                return start;
            }
        }
        start.initial_directory = folder
            .ancestors()
            .take_while(|_| !cancel.is_cancelled())
            .find(|parent| parent.is_dir())
            .map(Path::to_owned);
    }
    start
}

/// Safe local operations preserving native paths and leaving volume policy to each OS.
/// Call service methods on background workers; cancellation cannot interrupt an OS call.
pub struct LocalServices {
    next_entry_id: AtomicU64,
}

impl Default for LocalServices {
    fn default() -> Self {
        Self {
            next_entry_id: AtomicU64::new(1),
        }
    }
}

impl LocalServices {
    /// Enumerate once and let a platform classify the cached, non-following file type.
    pub fn read_directory_with(
        &self,
        path: &Path,
        cancel: &Cancellation,
        classify: impl Fn(fs::FileType) -> EntryKind,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        self.read_directory_with_metadata(path, cancel, classify, |_| None, emit)
    }

    /// Let a platform reuse metadata already returned by native enumeration.
    /// The callback must not follow links or open file contents.
    ///
    /// An entry that vanished during enumeration is left out silently. An entry whose
    /// type cannot be read (Linux reads it with `lstat` when the directory record has
    /// no type) is left out too, but its siblings are still listed: the first such
    /// error, naming how many entries were left out, is returned once the listing has
    /// ended, so a caller (a folder-size scan, a listing) knows the result is
    /// incomplete. A failure of the enumeration itself ends it at once.
    pub fn read_directory_with_metadata(
        &self,
        path: &Path,
        cancel: &Cancellation,
        classify: impl Fn(fs::FileType) -> EntryKind,
        metadata: impl Fn(&fs::DirEntry) -> Option<EntryMetadata>,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        if cancel.is_cancelled() {
            return Ok(());
        }
        let directory =
            fs::read_dir(path).map_err(|error| FsError::from_io("read directory", path, error))?;
        list_items(
            path,
            directory,
            cancel,
            // DirEntry::file_type uses directory enumeration data where available;
            // it does not follow symlinks or fetch full per-entry metadata.
            |item| item.file_type().map(&classify),
            fs::DirEntry::path,
            metadata,
            &self.next_entry_id,
            emit,
        )
    }
}

/// Entries left out of one listing because their type could not be read: a count
/// and the first error, never a list.
#[derive(Default)]
struct SkippedEntries {
    count: u64,
    first: Option<FsError>,
}

impl SkippedEntries {
    fn record(&mut self, path: &Path, error: io::Error) {
        self.count = self.count.saturating_add(1);
        if self.first.is_none() {
            self.first = Some(FsError::from_io("read entry type", path, error));
        }
    }

    fn into_result(self) -> Result<(), FsError> {
        let Some(mut error) = self.first else {
            return Ok(());
        };
        if self.count > 1 {
            error.message = format!(
                "{} ({} entries of this folder could not be read and were left out)",
                error.message, self.count
            );
        }
        Err(error)
    }
}

/// One listing pass over native enumeration `items`, generic so that tests can
/// inject per-entry failures.
#[allow(clippy::too_many_arguments)]
fn list_items<D>(
    path: &Path,
    items: impl Iterator<Item = io::Result<D>>,
    cancel: &Cancellation,
    kind_of: impl Fn(&D) -> io::Result<EntryKind>,
    path_of: impl Fn(&D) -> std::path::PathBuf,
    metadata: impl Fn(&D) -> Option<EntryMetadata>,
    next_entry_id: &AtomicU64,
    emit: &mut dyn FnMut(Entry) -> bool,
) -> Result<(), FsError> {
    let mut skipped = SkippedEntries::default();
    for item in items {
        if cancel.is_cancelled() {
            break;
        }
        let item = match item {
            Ok(item) => item,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            // The enumeration itself failed: Linux ends the stream after a `readdir`
            // error and Windows would repeat a failed `FindNextFileW`.
            Err(error) => return Err(FsError::from_io("read directory entry", path, error)),
        };
        let kind = match kind_of(&item) {
            Ok(kind) => kind,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                skipped.record(&path_of(&item), error);
                continue;
            }
        };
        if cancel.is_cancelled() {
            break;
        }
        let id = EntryId(next_entry_id.fetch_add(1, Ordering::Relaxed));
        let mut entry = Entry::new(id, path_of(&item), kind);
        if let Some(value) = metadata(&item) {
            entry.metadata = value;
        }
        if !emit(entry) {
            break;
        }
    }
    skipped.into_result()
}

impl DirectoryService for LocalServices {
    fn read_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        self.read_directory_with(path, cancel, classify, emit)
    }

    fn entry_metadata(&self, path: &Path) -> Result<EntryMetadata, FsError> {
        // Do not open targets: dangling links, FIFOs and devices are safe to inspect.
        let metadata = fs::symlink_metadata(path)
            .map_err(|error| FsError::from_io("read metadata", path, error))?;
        Ok(EntryMetadata {
            size: metadata.is_file().then_some(metadata.len()),
            modified: metadata.modified().ok(),
            ..EntryMetadata::default()
        })
    }
}

fn classify(file_type: fs::FileType) -> EntryKind {
    if file_type.is_symlink() {
        EntryKind::Symlink
    } else if file_type.is_dir() {
        EntryKind::Directory
    } else if file_type.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    }
}

/// Unix capacity from `statvfs`; Windows adapters query their volumes natively.
#[cfg(unix)]
impl midnight_core::StorageService for LocalServices {
    fn space_info(&self, path: &Path) -> Result<midnight_core::SpaceInfo, FsError> {
        let stats = rustix::fs::statvfs(path).map_err(|error| {
            FsError::from_io("read filesystem capacity", path, io::Error::from(error))
        })?;
        Ok(midnight_core::SpaceInfo {
            // Space available to unprivileged users, in fragment-size units.
            available: stats.f_bavail.saturating_mul(stats.f_frsize),
            total: stats.f_blocks.saturating_mul(stats.f_frsize),
        })
    }
}

#[cfg(all(test, unix))]
mod capacity_tests {
    use super::LocalServices;
    use midnight_core::StorageService;

    #[test]
    fn capacity_is_reported_for_existing_paths_and_fails_for_missing_ones() {
        let root = tempfile::tempdir().unwrap();
        let space = LocalServices::default().space_info(root.path()).unwrap();
        assert!(space.total > 0);
        assert!(space.available <= space.total);
        let missing = LocalServices::default().space_info(&root.path().join("missing"));
        assert_eq!(missing.unwrap_err().kind, std::io::ErrorKind::NotFound);
    }
}

#[cfg(test)]
mod listing_tests {
    use super::*;
    use std::path::PathBuf;

    /// One enumerated record whose type read can fail.
    struct Item(&'static str, Option<io::ErrorKind>);

    fn listed(
        items: Vec<io::Result<Item>>,
        cancel: &Cancellation,
    ) -> (Vec<PathBuf>, Result<(), FsError>) {
        let mut names = Vec::new();
        let result = list_items(
            Path::new("folder"),
            items.into_iter(),
            cancel,
            |item| item.1.map_or(Ok(EntryKind::File), |kind| Err(kind.into())),
            |item| Path::new("folder").join(item.0),
            |_| None,
            &AtomicU64::new(1),
            &mut |entry| {
                names.push(entry.path);
                true
            },
        );
        (names, result)
    }

    #[test]
    fn an_unreadable_entry_type_is_left_out_and_its_siblings_are_still_listed() {
        let (names, result) = listed(
            vec![
                Ok(Item("before", None)),
                Ok(Item("denied", Some(io::ErrorKind::PermissionDenied))),
                Ok(Item("gone", Some(io::ErrorKind::NotFound))),
                Ok(Item("broken", Some(io::ErrorKind::Other))),
                Ok(Item("after", None)),
            ],
            &Cancellation::default(),
        );
        assert_eq!(
            names,
            [Path::new("folder/before"), Path::new("folder/after")]
        );
        // The first failure is named, with how many entries were left out; an entry
        // that vanished is not a failure.
        let error = result.unwrap_err();
        assert_eq!(error.kind, io::ErrorKind::PermissionDenied);
        assert_eq!(error.path, Path::new("folder/denied"));
        assert_eq!(error.operation, "read entry type");
        assert!(
            error
                .message
                .contains("(2 entries of this folder could not be read")
        );
    }

    #[test]
    fn a_single_unreadable_entry_keeps_its_own_message_and_vanished_entries_are_no_error() {
        let (names, result) = listed(
            vec![
                Ok(Item("denied", Some(io::ErrorKind::PermissionDenied))),
                Ok(Item("after", None)),
            ],
            &Cancellation::default(),
        );
        assert_eq!(names, [Path::new("folder/after")]);
        assert!(
            !result
                .unwrap_err()
                .message
                .contains("entries of this folder")
        );
        let (names, result) = listed(
            vec![
                Err(io::ErrorKind::NotFound.into()),
                Ok(Item("gone", Some(io::ErrorKind::NotFound))),
                Ok(Item("kept", None)),
            ],
            &Cancellation::default(),
        );
        assert_eq!(names, [Path::new("folder/kept")]);
        assert!(result.is_ok());
    }

    #[test]
    fn a_failed_enumeration_still_ends_the_listing() {
        let (names, result) = listed(
            vec![
                Ok(Item("first", None)),
                Err(io::ErrorKind::Other.into()),
                Ok(Item("unreached", None)),
            ],
            &Cancellation::default(),
        );
        assert_eq!(names, [Path::new("folder/first")]);
        assert_eq!(result.unwrap_err().operation, "read directory entry");
    }

    #[test]
    fn cancellation_stops_the_listing_without_reading_more_types() {
        let cancel = Cancellation::default();
        cancel.cancel();
        let (names, result) = listed(vec![Ok(Item("first", None))], &cancel);
        assert!(names.is_empty());
        assert!(result.is_ok());
    }
}
