//! Delete of native paths that a Find captured, for a result list fed to a panel.
//! It enters the ordinary captured-Delete preparation, confirmation and progress
//! flow. Each target is pinned by the mutation service on a backend worker and
//! must still be the kind (and, for a file, the size) that the search found; the
//! stamp and request only identify this capture, they are no tree session.
use crate::*;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

/// One captured target: native path, kind and, for a file, its size.
#[derive(Clone, Debug)]
pub struct CapturedPath {
    pub path: PathBuf,
    pub kind: EntryKind,
    pub size: Option<u64>,
}

#[derive(Debug)]
pub struct CapturedPathsDelete {
    stamp: TreeViewStamp,
    request: TreeRequestId,
    mode: DeleteMode,
    targets: Box<[CapturedPath]>,
}

impl CapturedPathsDelete {
    /// `serial` must be positive and unique per capture. Targets inside another
    /// target are dropped: deleting a folder deletes its contents. Only absolute
    /// native files and folders are accepted, at most MAX_CAPTURED_DELETE_ROOTS.
    pub fn new(
        pane: PaneId,
        serial: u64,
        mode: DeleteMode,
        targets: Vec<CapturedPath>,
    ) -> Result<Self, &'static str> {
        if serial == 0 {
            return Err("A delete capture needs a positive serial.");
        }
        if targets.iter().any(|target| {
            !target.path.is_absolute()
                || target.path.file_name().is_none()
                || !matches!(target.kind, EntryKind::File | EntryKind::Directory)
        }) {
            return Err("Only absolute native files and folders can be deleted here.");
        }
        let mut sorted: Vec<&CapturedPath> = targets.iter().collect();
        sorted.sort_by(|a, b| a.path.cmp(&b.path));
        let mut kept: Vec<CapturedPath> = Vec::with_capacity(sorted.len());
        for target in sorted {
            // Sorted by path, a folder precedes everything inside it.
            if kept
                .last()
                .is_some_and(|last| last.path == target.path || inside(&target.path, &last.path))
            {
                continue;
            }
            kept.push(target.clone());
        }
        if kept.is_empty() {
            return Err("Nothing is selected to delete.");
        }
        if kept.len() > MAX_CAPTURED_DELETE_ROOTS {
            return Err("At most 4096 items can be deleted at once from search results.");
        }
        Ok(Self {
            stamp: TreeViewStamp {
                pane,
                navigation: NavigationId(serial),
                session: TreeSessionId(serial),
                revision: TreeRevision(1),
            },
            request: TreeRequestId(serial),
            mode,
            targets: kept.into(),
        })
    }

    pub fn targets(&self) -> &[CapturedPath] {
        &self.targets
    }
}

fn inside(path: &Path, folder: &Path) -> bool {
    path != folder && path.starts_with(folder)
}

impl CapturedDeleteSources for CapturedPathsDelete {
    fn stamp(&self) -> TreeViewStamp {
        self.stamp
    }
    fn request(&self) -> TreeRequestId {
        self.request
    }
    fn mode(&self) -> DeleteMode {
        self.mode
    }
    fn root_count(&self) -> usize {
        self.targets.len()
    }
    fn prepare(
        &self,
        service: &dyn MutationService,
        cancel: &Cancellation,
    ) -> Result<Vec<Arc<dyn PreparedMutation>>, FsError> {
        let mut prepared = Vec::with_capacity(self.targets.len());
        for target in self.targets.iter() {
            if cancel.is_cancelled() {
                return Err(FsError::from_io(
                    "prepare delete",
                    &target.path,
                    std::io::ErrorKind::Interrupted.into(),
                ));
            }
            let pinned = service.prepare(&target.path, cancel)?;
            let item = pinned.item();
            let same = item.path == target.path
                && item.kind == target.kind
                && (target.kind != EntryKind::File
                    || target.size.is_none()
                    || item.metadata.size == target.size);
            if !same {
                return Err(FsError::from_io(
                    "prepare delete",
                    &target.path,
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "This item changed after the search found it. Search again before deleting it.",
                    ),
                ));
            }
            prepared.push(pinned);
        }
        Ok(prepared)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An absolute path below the temporary folder on every platform.
    fn absolute(relative: &str) -> PathBuf {
        relative
            .split('/')
            .fold(std::env::temp_dir().join("data"), |path, part| {
                path.join(part)
            })
    }

    fn target(path: &str, kind: EntryKind) -> CapturedPath {
        CapturedPath {
            path: absolute(path),
            kind,
            size: None,
        }
    }

    #[test]
    fn nested_targets_are_covered_by_their_folder_and_bounds_hold() {
        let capture = CapturedPathsDelete::new(
            PaneId::Left,
            3,
            DeleteMode::Trash,
            vec![
                target("a/file.txt", EntryKind::File),
                target("a", EntryKind::Directory),
                target("ab.txt", EntryKind::File),
                target("a/deeper/x", EntryKind::File),
            ],
        )
        .unwrap();
        let kept: Vec<_> = capture.targets().iter().map(|t| t.path.clone()).collect();
        assert_eq!(kept, [absolute("a"), absolute("ab.txt")]);
        assert_eq!(capture.root_count(), 2);
        assert_eq!(capture.request(), TreeRequestId(3));
        assert!(CapturedPathsDelete::new(PaneId::Left, 0, DeleteMode::Trash, vec![]).is_err());
        assert!(
            CapturedPathsDelete::new(
                PaneId::Left,
                1,
                DeleteMode::Trash,
                vec![CapturedPath {
                    path: PathBuf::from("relative"),
                    kind: EntryKind::File,
                    size: None,
                }]
            )
            .is_err()
        );
    }
}
