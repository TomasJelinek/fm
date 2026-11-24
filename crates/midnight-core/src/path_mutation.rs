//! Captured native Find results enter the existing Rename and metadata pipelines.
//! Construction is bounded and performs no I/O; only backend preparation opens targets.
use crate::*;
use std::{
    collections::HashSet,
    path::{Component, Path},
    sync::Arc,
};

#[derive(Debug)]
pub struct CapturedPathRename {
    stamp: TreeViewStamp,
    request: TreeRequestId,
    target: CapturedPath,
}

impl CapturedPathRename {
    /// One focused native result, independent of the feed's marked selection.
    pub fn new(pane: PaneId, serial: u64, target: CapturedPath) -> Result<Self, &'static str> {
        validate_target(&target)?;
        Ok(Self {
            stamp: stamp(pane, serial)?,
            request: TreeRequestId(serial),
            target,
        })
    }

    pub fn target(&self) -> &CapturedPath {
        &self.target
    }
}

impl CapturedRenameTarget for CapturedPathRename {
    fn stamp(&self) -> TreeViewStamp {
        self.stamp
    }
    fn request(&self) -> TreeRequestId {
        self.request
    }
    fn row(&self) -> TreeRowId {
        TreeRowId(1)
    }
    fn prepare(
        &self,
        service: &dyn MutationService,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn PreparedMutation>, FsError> {
        prepare_target(&self.target, service, cancel)
    }
}

#[derive(Debug)]
pub struct CapturedPathsMetadata {
    stamp: TreeViewStamp,
    request: TreeRequestId,
    targets: Box<[CapturedPath]>,
    covered: Box<[bool]>,
}

impl CapturedPathsMetadata {
    /// At most 256 distinct native targets. Descendants precede their ancestors;
    /// coverage only skips repeated recursive traversal, never nonrecursive edits.
    pub fn new(
        pane: PaneId,
        serial: u64,
        mut targets: Vec<CapturedPath>,
    ) -> Result<Self, &'static str> {
        let stamp = stamp(pane, serial)?;
        if targets.is_empty() || targets.len() > MAX_METADATA_TARGETS {
            return Err("Select between 1 and 256 search results to edit metadata.");
        }
        let mut seen = HashSet::with_capacity(targets.len());
        for target in &targets {
            validate_target(target)?;
            if !seen.insert(&target.path) {
                return Err("Captured metadata paths must be unique.");
            }
        }
        targets.sort_by(|a, b| {
            b.path
                .components()
                .count()
                .cmp(&a.path.components().count())
                .then_with(|| a.path.cmp(&b.path))
        });
        let directories: HashSet<&Path> = targets
            .iter()
            .filter(|target| target.kind == EntryKind::Directory)
            .map(|target| target.path.as_path())
            .collect();
        let covered = targets
            .iter()
            .map(|target| {
                target
                    .path
                    .ancestors()
                    .skip(1)
                    .any(|parent| directories.contains(parent))
            })
            .collect();
        Ok(Self {
            stamp,
            request: TreeRequestId(serial),
            targets: targets.into(),
            covered,
        })
    }

    pub fn targets(&self) -> &[CapturedPath] {
        &self.targets
    }
}

impl CapturedMetadataSources for CapturedPathsMetadata {
    fn stamp(&self) -> TreeViewStamp {
        self.stamp
    }
    fn request(&self) -> TreeRequestId {
        self.request
    }
    fn root_count(&self) -> usize {
        self.targets.len()
    }
    fn covered_roots(&self) -> &[bool] {
        &self.covered
    }
    fn prepare(
        &self,
        service: &dyn MutationService,
        cancel: &Cancellation,
    ) -> Result<Vec<Arc<dyn PreparedMutation>>, FsError> {
        self.targets
            .iter()
            .map(|target| prepare_target(target, service, cancel))
            .collect()
    }
}

fn stamp(pane: PaneId, serial: u64) -> Result<TreeViewStamp, &'static str> {
    if serial == 0 {
        return Err("A native mutation capture needs a positive serial.");
    }
    Ok(TreeViewStamp {
        pane,
        navigation: NavigationId(serial),
        session: TreeSessionId(serial),
        revision: TreeRevision(1),
    })
}

fn validate_target(target: &CapturedPath) -> Result<(), &'static str> {
    if !target.path.is_absolute()
        || target.path.file_name().is_none()
        || target.path.as_os_str().as_encoded_bytes().contains(&0)
        || target
            .path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        || !matches!(target.kind, EntryKind::File | EntryKind::Directory)
    {
        return Err(
            "Only absolute native files and folders without NUL or parent steps are accepted.",
        );
    }
    Ok(())
}

fn prepare_target(
    target: &CapturedPath,
    service: &dyn MutationService,
    cancel: &Cancellation,
) -> Result<Arc<dyn PreparedMutation>, FsError> {
    if !cancel.wait_if_paused() {
        return Err(FsError::from_io(
            "prepare search result",
            &target.path,
            std::io::ErrorKind::Interrupted.into(),
        ));
    }
    let pinned = service.prepare(&target.path, cancel)?;
    let item = pinned.item();
    if item.path != target.path
        || item.kind != target.kind
        || (target.kind == EntryKind::File
            && target.size.is_some()
            && item.metadata.size != target.size)
    {
        return Err(FsError::from_io(
            "prepare search result",
            &target.path,
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "This item changed after the search found it. Search again before changing it.",
            ),
        ));
    }
    Ok(pinned)
}

#[cfg(test)]
#[path = "path_mutation_tests.rs"]
mod tests;
