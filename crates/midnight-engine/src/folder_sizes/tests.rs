use super::*;
use midnight_core::{
    DirectoryService, EntryKind, EntryMetadata, FsError, SpaceInfo, StorageService, Volume,
    VolumeService,
};
use std::{
    collections::{HashMap, HashSet},
    io,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

#[derive(Default)]
pub(super) struct Tree {
    directories: HashMap<PathBuf, Vec<Entry>>,
    links: HashMap<PathBuf, PathBuf>,
    lengths: HashMap<PathBuf, u64>,
    pub(super) unreadable: HashSet<PathBuf>,
    reads: Mutex<Vec<PathBuf>>,
    metadata_reads: Mutex<Vec<PathBuf>>,
    pub(super) cancel_on_metadata: Option<Cancellation>,
    metadata_delay: Duration,
}

impl Tree {
    pub(super) fn directory(&mut self, path: &str, entries: Vec<Entry>) {
        self.directories.insert(path.into(), entries);
    }

    pub(super) fn length(&mut self, path: &str, bytes: u64) {
        self.lengths.insert(path.into(), bytes);
    }
}

impl DirectoryService for Tree {
    fn resolve_size_directory(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Option<SizeDirectoryTarget>, FsError> {
        use std::hash::{Hash, Hasher};
        let resolved = self.links.get(path).map_or(path, PathBuf::as_path);
        if self.unreadable.contains(resolved) {
            return Err(FsError::from_io(
                "resolve directory size target",
                path,
                io::ErrorKind::PermissionDenied.into(),
            ));
        }
        if !self.directories.contains_key(resolved) {
            return Ok(None);
        }
        let mut identity = std::collections::hash_map::DefaultHasher::new();
        resolved.hash(&mut identity);
        Ok(Some(SizeDirectoryTarget {
            path: resolved.to_owned(),
            identity: SizeDirectoryIdentity {
                device: 1,
                object: u128::from(identity.finish()).to_le_bytes(),
            },
        }))
    }
    fn read_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        self.reads.lock().unwrap().push(path.into());
        if self.unreadable.contains(path) {
            return Err(FsError::from_io(
                "test read directory",
                path,
                io::ErrorKind::PermissionDenied.into(),
            ));
        }
        let entries = self.directories.get(path).ok_or_else(|| {
            FsError::from_io("test read directory", path, io::ErrorKind::NotFound.into())
        })?;
        for entry in entries {
            if cancel.is_cancelled() || !emit(entry.clone()) {
                break;
            }
        }
        Ok(())
    }

    fn entry_metadata(&self, path: &Path) -> Result<EntryMetadata, FsError> {
        self.metadata_reads.lock().unwrap().push(path.into());
        if !self.metadata_delay.is_zero() {
            std::thread::sleep(self.metadata_delay);
        }
        if let Some(cancel) = &self.cancel_on_metadata {
            cancel.cancel();
        }
        self.lengths
            .get(path)
            .copied()
            .map(|size| EntryMetadata {
                size: Some(size),
                modified: None,
                ..EntryMetadata::default()
            })
            .ok_or_else(|| FsError::from_io("test metadata", path, io::ErrorKind::NotFound.into()))
    }
}

impl VolumeService for Tree {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        unreachable!("A size scan must not discover volumes")
    }
}

impl StorageService for Tree {
    fn space_info(&self, _: &Path) -> Result<SpaceInfo, FsError> {
        unreachable!("Logical lengths must not query allocation or capacity")
    }
}

pub(super) fn item(id: u64, path: &str, kind: EntryKind) -> Entry {
    Entry::new(EntryId(id), path.into(), kind)
}

fn finished(tree: &Tree, roots: &[Entry]) -> Vec<(EntryId, FolderSize)> {
    let mut results = vec![];
    compute(
        tree,
        roots,
        &Cancellation::default(),
        &mut |id, mut size| {
            if size.finished {
                // Scalar tests remain independent of opaque report identity, while
                // every counted error must still retain a diagnostic handle.
                assert_eq!(
                    size.warnings
                        .as_ref()
                        .map_or(0, |report| report.summary().observed),
                    size.errors
                );
                size.warnings = None;
                results.push((id, size));
            }
            true
        },
    );
    results
}

#[test]
fn nested_unicode_directories_sum_regular_file_lengths_and_empty_directory_is_zero() {
    let mut tree = Tree::default();
    tree.directory(
        "root/文档",
        vec![
            item(2, "root/文档/Žluťoučký.txt", EntryKind::File),
            item(3, "root/文档/👩🏽‍💻", EntryKind::Directory),
        ],
    );
    tree.directory(
        "root/文档/👩🏽‍💻",
        vec![item(4, "root/文档/👩🏽‍💻/مرحبا.txt", EntryKind::File)],
    );
    tree.directory("root/empty", vec![]);
    tree.length("root/文档/Žluťoučký.txt", 7);
    tree.length("root/文档/👩🏽‍💻/مرحبا.txt", 11);

    let results = finished(
        &tree,
        &[
            item(1, "root/文档", EntryKind::Directory),
            item(5, "root/empty", EntryKind::Directory),
            item(6, "root/file", EntryKind::File),
        ],
    );

    assert_eq!(
        results,
        vec![
            (
                EntryId(1),
                FolderSize {
                    bytes: 18,
                    files: 2,
                    finished: true,
                    ..FolderSize::default()
                }
            ),
            (
                EntryId(5),
                FolderSize {
                    finished: true,
                    ..FolderSize::default()
                }
            ),
        ]
    );
    assert_eq!(tree.metadata_reads.lock().unwrap().len(), 2);
}

#[test]
fn non_directory_symlinks_and_special_files_are_not_opened() {
    let mut tree = Tree::default();
    tree.directory(
        "root/dir",
        vec![
            item(2, "root/dir/cycle", EntryKind::Symlink),
            item(3, "root/dir/device", EntryKind::Other),
        ],
    );
    let results = finished(
        &tree,
        &[
            item(1, "root/dir", EntryKind::Directory),
            item(4, "root/link", EntryKind::Symlink),
        ],
    );
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].1,
        FolderSize {
            skipped_links: 1,
            finished: true,
            ..FolderSize::default()
        }
    );
    assert_eq!(*tree.reads.lock().unwrap(), vec![PathBuf::from("root/dir")]);
    assert!(tree.metadata_reads.lock().unwrap().is_empty());
}

#[test]
fn directory_links_count_target_contents_in_recursive_and_explicit_root_scans() {
    let mut tree = Tree::default();
    tree.directory(
        "root/dir",
        vec![item(2, "root/dir/junction", EntryKind::DirectoryLink)],
    );
    tree.directory(
        "root/dir/junction",
        vec![item(3, "root/dir/junction/payload", EntryKind::File)],
    );
    tree.length("root/dir/junction/payload", 999);
    tree.links
        .insert("root/direct-link".into(), "root/dir/junction".into());
    let result = finished(
        &tree,
        &[
            item(1, "root/dir", EntryKind::Directory),
            item(4, "root/direct-link", EntryKind::DirectoryLink),
        ],
    );
    assert_eq!(
        result,
        [
            (
                EntryId(1),
                FolderSize {
                    bytes: 999,
                    files: 1,
                    finished: true,
                    ..FolderSize::default()
                }
            ),
            (
                EntryId(4),
                FolderSize {
                    bytes: 999,
                    files: 1,
                    finished: true,
                    ..FolderSize::default()
                }
            )
        ]
    );
    assert_eq!(tree.metadata_reads.lock().unwrap().len(), 2);
}

#[test]
fn size_link_ancestor_and_self_cycles_stop_before_recounting_files() {
    let mut tree = Tree::default();
    tree.directory(
        "root",
        vec![
            item(2, "root/file", EntryKind::File),
            item(3, "root/self", EntryKind::DirectoryLink),
            item(4, "root/child", EntryKind::Directory),
        ],
    );
    tree.directory(
        "root/child",
        vec![
            item(5, "root/child/file", EntryKind::File),
            item(6, "root/child/up", EntryKind::DirectoryLink),
        ],
    );
    tree.links.insert("root/self".into(), "root".into());
    tree.links.insert("root/child/up".into(), "root".into());
    tree.length("root/file", 7);
    tree.length("root/child/file", 11);
    let results = finished(&tree, &[item(1, "root", EntryKind::Directory)]);
    assert_eq!(
        results[0].1,
        FolderSize {
            bytes: 18,
            files: 2,
            skipped_links: 2,
            finished: true,
            ..FolderSize::default()
        }
    );
    assert_eq!(
        tree.reads.lock().unwrap().len(),
        2,
        "cycles are never enumerated again"
    );
}

#[test]
fn size_link_noncyclic_aliases_keep_their_independent_logical_contents() {
    let mut tree = Tree::default();
    tree.directory(
        "root",
        vec![
            item(2, "root/first", EntryKind::DirectoryLink),
            item(3, "root/second", EntryKind::DirectoryLink),
        ],
    );
    tree.directory(
        "elsewhere",
        vec![item(4, "elsewhere/file", EntryKind::File)],
    );
    tree.links.insert("root/first".into(), "elsewhere".into());
    tree.links.insert("root/second".into(), "elsewhere".into());
    tree.length("elsewhere/file", 5);
    let results = finished(&tree, &[item(1, "root", EntryKind::Directory)]);
    assert_eq!(results[0].1.bytes, 10);
    assert_eq!(results[0].1.files, 2);
    assert_eq!(results[0].1.skipped_links, 0);
    assert_eq!(results[0].1.errors, 0);
}

#[test]
fn size_link_inaccessible_target_keeps_partial_bytes_and_warning_details() {
    let mut tree = Tree::default();
    tree.directory(
        "root",
        vec![
            item(2, "root/file", EntryKind::File),
            item(3, "root/denied", EntryKind::DirectoryLink),
        ],
    );
    tree.length("root/file", 7);
    tree.links
        .insert("root/denied".into(), "inaccessible".into());
    tree.unreadable.insert("inaccessible".into());
    let mut last = None;
    compute(
        &tree,
        &[item(1, "root", EntryKind::Directory)],
        &Cancellation::default(),
        &mut |_, size| {
            last = Some(size);
            true
        },
    );
    let size = last.unwrap();
    assert!(size.finished);
    assert_eq!(
        (size.bytes, size.files, size.errors, size.skipped_links),
        (7, 1, 1, 1)
    );
    let warnings = size.warnings.unwrap();
    assert_eq!(warnings.summary().observed, 1);
    assert!(warnings.summary().tooltip.contains("PermissionDenied"));
    assert!(warnings.summary().tooltip.contains("denied"));
}

#[test]
fn unreadable_subdirectory_and_vanished_file_produce_explicit_partial_result() {
    let mut tree = Tree::default();
    tree.directory(
        "root/dir",
        vec![
            item(2, "root/dir/good", EntryKind::File),
            item(3, "root/dir/gone", EntryKind::File),
            item(4, "root/dir/private", EntryKind::Directory),
        ],
    );
    tree.length("root/dir/good", 13);
    tree.unreadable.insert("root/dir/private".into());
    let result = finished(&tree, &[item(1, "root/dir", EntryKind::Directory)]);
    assert_eq!(
        result[0].1,
        FolderSize {
            bytes: 13,
            files: 1,
            errors: 2,
            finished: true,
            ..FolderSize::default()
        }
    );
}

#[test]
fn cancellation_during_metadata_stops_before_more_entries_or_final_success() {
    let cancel = Cancellation::default();
    let mut tree = Tree {
        cancel_on_metadata: Some(cancel.clone()),
        ..Tree::default()
    };
    tree.directory(
        "root/dir",
        vec![
            item(2, "root/dir/a", EntryKind::File),
            item(3, "root/dir/b", EntryKind::File),
        ],
    );
    tree.length("root/dir/a", 10);
    tree.length("root/dir/b", 20);
    let mut finals = vec![];
    compute(
        &tree,
        &[item(1, "root/dir", EntryKind::Directory)],
        &cancel,
        &mut |id, size| {
            if size.finished {
                finals.push((id, size));
            }
            true
        },
    );
    assert!(finals.is_empty());
    assert_eq!(tree.metadata_reads.lock().unwrap().len(), 1);
}

#[test]
fn receiver_rejection_stops_scan_without_starting_next_root() {
    let mut tree = Tree::default();
    tree.directory("root/first", vec![]);
    tree.directory("root/second", vec![]);
    let mut events = 0;
    compute(
        &tree,
        &[
            item(1, "root/first", EntryKind::Directory),
            item(2, "root/second", EntryKind::Directory),
        ],
        &Cancellation::default(),
        &mut |_, size| {
            events += 1;
            !size.finished
        },
    );
    assert!(events > 0);
    assert_eq!(
        *tree.reads.lock().unwrap(),
        vec![PathBuf::from("root/first")]
    );
}

#[test]
fn size_overflow_saturates_and_marks_result_partial() {
    let mut tree = Tree::default();
    tree.directory(
        "root/dir",
        vec![
            item(2, "root/dir/a", EntryKind::File),
            item(3, "root/dir/b", EntryKind::File),
        ],
    );
    tree.length("root/dir/a", u64::MAX);
    tree.length("root/dir/b", 1);
    let result = finished(&tree, &[item(1, "root/dir", EntryKind::Directory)]);
    assert_eq!(
        result[0].1,
        FolderSize {
            bytes: u64::MAX,
            files: 2,
            errors: 1,
            finished: true,
            ..FolderSize::default()
        }
    );
}

#[test]
fn invalid_provider_paths_cannot_escape_or_reenter_parent() {
    let mut tree = Tree::default();
    tree.directory(
        "root/dir",
        vec![
            item(2, "root/dir", EntryKind::Directory),
            item(3, "root/dir/../escape", EntryKind::Directory),
            item(4, "outside/file", EntryKind::File),
            item(5, "root/dir/nested/child", EntryKind::Directory),
        ],
    );
    let result = finished(&tree, &[item(1, "root/dir", EntryKind::Directory)]);
    assert_eq!(result[0].1.errors, 4);
    assert_eq!(*tree.reads.lock().unwrap(), vec![PathBuf::from("root/dir")]);
    assert!(tree.metadata_reads.lock().unwrap().is_empty());
}

#[test]
fn a_valid_root_containing_parent_components_preserves_native_path_spelling() {
    let mut tree = Tree::default();
    tree.directory(
        "root/start/../dir",
        vec![item(2, "root/start/../dir/a", EntryKind::File)],
    );
    tree.length("root/start/../dir/a", 23);
    let results = finished(&tree, &[item(1, "root/start/../dir", EntryKind::Directory)]);
    assert_eq!(
        results[0].1,
        FolderSize {
            bytes: 23,
            files: 1,
            finished: true,
            ..FolderSize::default()
        }
    );
    assert_eq!(
        *tree.metadata_reads.lock().unwrap(),
        vec![PathBuf::from("root/start/../dir/a")]
    );
}

#[test]
fn already_cancelled_scan_does_not_touch_provider() {
    let cancel = Cancellation::default();
    cancel.cancel();
    let tree = Tree::default();
    compute(
        &tree,
        &[item(1, "root/dir", EntryKind::Directory)],
        &cancel,
        &mut |_, _| panic!("Cancelled work must not emit a successful result"),
    );
    assert!(tree.reads.lock().unwrap().is_empty());
}

#[test]
fn frontier_budget_reports_omitted_subtree_as_partial() {
    let mut tree = Tree::default();
    tree.directory(
        "root/dir",
        vec![
            item(2, "root/dir/first", EntryKind::Directory),
            item(3, "root/dir/second", EntryKind::Directory),
        ],
    );
    tree.directory(
        "root/dir/first",
        vec![item(4, "root/dir/first/a", EntryKind::File)],
    );
    tree.length("root/dir/first/a", 17);
    let mut results = vec![];
    compute_with_budget(
        &tree,
        &[item(1, "root/dir", EntryKind::Directory)],
        &Cancellation::default(),
        &mut |_, mut size| {
            if size.finished {
                assert_eq!(size.warnings.as_ref().unwrap().summary().observed, 1);
                size.warnings = None;
                results.push(size);
            }
            true
        },
        0,
    );
    assert_eq!(
        results,
        vec![FolderSize {
            bytes: 17,
            files: 1,
            errors: 1,
            finished: true,
            ..FolderSize::default()
        }]
    );
    assert!(
        !tree
            .reads
            .lock()
            .unwrap()
            .contains(&PathBuf::from("root/dir/second"))
    );
}

#[test]
fn frontier_budget_does_not_limit_one_native_path_length() {
    let mut tree = Tree::default();
    let long_name = "文档".repeat(20_000);
    let child = format!("root/dir/{long_name}");
    tree.directory("root/dir", vec![item(2, &child, EntryKind::Directory)]);
    tree.directory(&child, vec![]);
    let mut result = None;
    compute_with_budget(
        &tree,
        &[item(1, "root/dir", EntryKind::Directory)],
        &Cancellation::default(),
        &mut |_, size| {
            if size.finished {
                result = Some(size);
            }
            true
        },
        1,
    );
    assert_eq!(
        result,
        Some(FolderSize {
            finished: true,
            ..FolderSize::default()
        })
    );
    assert!(tree.reads.lock().unwrap().contains(&PathBuf::from(child)));
}

#[test]
fn a_closed_progress_receiver_prevents_all_filesystem_calls() {
    let tree = Tree::default();
    compute(
        &tree,
        &[item(1, "root/dir", EntryKind::Directory)],
        &Cancellation::default(),
        &mut |_, _| false,
    );
    assert!(tree.reads.lock().unwrap().is_empty());
}

#[test]
fn slow_scan_emits_intermediate_progress_at_most_four_times_per_second() {
    let mut tree = Tree {
        metadata_delay: PROGRESS_INTERVAL + Duration::from_millis(10),
        ..Tree::default()
    };
    tree.directory(
        "root/dir",
        vec![
            item(2, "root/dir/a", EntryKind::File),
            item(3, "root/dir/b", EntryKind::File),
        ],
    );
    tree.length("root/dir/a", 10);
    tree.length("root/dir/b", 20);
    let mut events = vec![];
    compute(
        &tree,
        &[item(1, "root/dir", EntryKind::Directory)],
        &Cancellation::default(),
        &mut |_, size| {
            events.push((Instant::now(), size));
            true
        },
    );
    assert!(
        events
            .iter()
            .any(|(_, size)| size.bytes == 10 && !size.finished)
    );
    assert!(
        events
            .iter()
            .any(|(_, size)| size.bytes == 30 && !size.finished)
    );
    let progress = events
        .iter()
        .filter(|(_, size)| !size.finished)
        .collect::<Vec<_>>();
    for pair in progress.windows(2) {
        assert!(pair[1].0.duration_since(pair[0].0) >= PROGRESS_INTERVAL);
    }
    assert!(events.last().unwrap().1.finished);
}
