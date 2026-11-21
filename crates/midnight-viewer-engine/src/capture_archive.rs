//! Archive members are content leases; native paths are never reconstructed.
use crate::capture::{CaptureEntry, ORDER_BUDGET, Order};
use midnight_core::{ArchiveMount, Cancellation, EntryKind, ViewerLaunch};
use midnight_plugin_host::{CapturedFile, ContentResources, ContentSource, Error};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// Charged per member in addition to its label and name copies: the descriptor, label
/// header, allocations and slack for the per-entry slot.
const ARCHIVE_ENTRY: usize = 128;

/// The retained order of one member; its content lease is created only when opened.
struct Descriptor {
    entry: u64,
    parent: u64,
    length: Option<u64>,
    name: String,
}

#[derive(Debug)]
struct Member {
    mount: Arc<dyn ArchiveMount>,
    entry: u64,
    parent: u64,
    length: u64,
    name: String,
    version: Vec<u8>,
    closed: Arc<AtomicBool>,
}
fn capture_member(source: Arc<Member>) -> Result<CapturedFile, Error> {
    let captured = CapturedFile::from_source(source.clone())?;
    Ok(captured.with_resources(Arc::new(Resources { main: source })))
}

#[derive(Debug)]
struct Resources {
    main: Arc<Member>,
}

impl ContentResources for Resources {
    fn capture(&self, segments: &[String]) -> Result<CapturedFile, Error> {
        crate::content_resources::validate_segments(segments)?;
        self.main.validate()?;
        if self.main.parent == 0 {
            return Err(Error("Archive resource scope unavailable".into()));
        }
        let mut directory = self.main.parent;
        let mut visited = vec![directory];
        for (index, segment) in segments.iter().enumerate() {
            self.main.validate()?;
            let entries = self
                .main
                .mount
                .list(directory, &Cancellation::default())
                .map_err(|_| Error("Archive resource listing unavailable".into()))?;
            if entries.len() > 16_384
                || entries.iter().any(|node| {
                    node.id.mount != self.main.mount.id()
                        || node.id.entry == 0
                        || node.parent != directory
                })
            {
                return Err(Error(
                    "Invalid or oversized archive resource listing".into(),
                ));
            }
            let mut matches = entries
                .iter()
                .filter(|node| node.raw_name.as_ref() == segment.as_bytes());
            let node = matches
                .next()
                .ok_or_else(|| Error("Archive resource not found".into()))?;
            if matches.next().is_some() || visited.contains(&node.id.entry) {
                return Err(Error("Ambiguous or cyclic archive resource".into()));
            }
            if index + 1 == segments.len() {
                if node.kind != EntryKind::File || !node.readable {
                    return Err(Error(
                        "Archive resource is not a readable regular member".into(),
                    ));
                }
                self.main.validate()?;
                return CapturedFile::from_source(Arc::new(Member {
                    mount: self.main.mount.clone(),
                    entry: node.id.entry,
                    parent: directory,
                    length: node
                        .size
                        .ok_or_else(|| Error("Archive resource length unavailable".into()))?,
                    name: segment.clone(),
                    version: self.main.version.clone(),
                    closed: self.main.closed.clone(),
                }));
            }
            if node.kind != EntryKind::Directory {
                return Err(Error(
                    "Archive resource ancestor is not an ordinary directory".into(),
                ));
            }
            directory = node.id.entry;
            visited.push(directory);
        }
        Err(Error("Empty archive resource path".into()))
    }

    /// Gives up this reference; the last one releases its main member's lease.
    fn release_on_worker(self: Arc<Self>) -> Result<(), Error> {
        match Arc::into_inner(self) {
            Some(Resources { main }) => main.release_on_worker(),
            None => Ok(()),
        }
    }
}
impl ContentSource for Member {
    fn length(&self) -> u64 {
        self.length
    }
    fn display_name(&self) -> String {
        self.name.clone()
    }
    fn version(&self) -> Vec<u8> {
        [self.version.as_slice(), &self.entry.to_le_bytes()].concat()
    }
    fn validate(&self) -> Result<(), Error> {
        if self.closed.load(Ordering::Acquire) {
            return Err(Error("Viewer closed".into()));
        }
        self.mount.validate().map_err(|e| Error(e.to_string()))
    }
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        self.mount
            .read(self.entry, offset, length, &Cancellation::default())
            .map_err(|e| Error(e.to_string()))
    }

    /// Gives up this reference; the last one releases its mount reference through the
    /// mount's own protocol, which tears the mount down only if no tab or viewer holds it.
    fn release_on_worker(self: Arc<Self>) -> Result<(), Error> {
        match Arc::into_inner(self) {
            Some(member) => member
                .mount
                .release_on_worker()
                .map_err(|error| Error(error.to_string())),
            None => Ok(()),
        }
    }
}
pub(crate) fn prepare(launch: &ViewerLaunch, closed: &Arc<AtomicBool>) -> Result<Order, String> {
    let archive = launch.archive.as_ref().ok_or("Missing archive focus")?;
    let mut nodes = Vec::new();
    for entry in launch.entries.iter() {
        if closed.load(Ordering::Acquire) {
            return Err("Viewer closed".into());
        }
        let (Some(node), Some(row)) = (&entry.virtual_node, launch.view.by_id.get(&entry.id))
        else {
            continue;
        };
        if !node.readable || node.id.mount != archive.mount.id() {
            continue;
        }
        nodes.push((*row, node));
    }
    nodes.sort_unstable_by_key(|(row, _)| *row);
    let focus = nodes
        .iter()
        .position(|(_, node)| node.id == archive.focused)
        .ok_or("Archive member is unavailable")?;
    let range = crate::capture::window(nodes.len(), focus, ORDER_BUDGET, |index| {
        nodes[index]
            .1
            .display_name
            .len()
            .saturating_mul(2)
            .saturating_add(ARCHIVE_ENTRY)
    });
    let members: Arc<[Descriptor]> = nodes[range.clone()]
        .iter()
        .map(|(_, node)| Descriptor {
            entry: node.id.entry,
            parent: node.parent,
            length: node.size,
            name: node.display_name.clone(),
        })
        .collect();
    // These values are format-selection labels only, never opened as native paths.
    let files: Arc<[PathBuf]> = members
        .iter()
        .map(|member| PathBuf::from(&member.name))
        .collect();
    let current = focus - range.start;
    let (mount, version, lease) = (
        archive.mount.clone(),
        archive.source_version.clone(),
        closed.clone(),
    );
    let entry: CaptureEntry = Arc::new(move |index, _| {
        let member = members.get(index).ok_or("Archive member is unavailable")?;
        capture_member(Arc::new(Member {
            mount: mount.clone(),
            entry: member.entry,
            parent: member.parent,
            length: member.length.ok_or("Archive member size unavailable")?,
            name: member.name.clone(),
            version: version.clone(),
            closed: lease.clone(),
        }))
        .map_err(|error| error.to_string())
    });
    let focused = entry(current, closed);
    Ok(Order {
        capture: (files.len() > 1).then_some(entry),
        files,
        current,
        focused,
    })
}

#[cfg(test)]
#[path = "archive_resources_tests.rs"]
mod resource_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use midnight_core::*;
    use std::sync::atomic::AtomicUsize;
    #[derive(Debug, Default)]
    struct Mount {
        changed: AtomicBool,
        validations: AtomicUsize,
        releases: AtomicUsize,
    }
    impl ArchiveMount for Mount {
        fn id(&self) -> u64 {
            8
        }
        /// The test keeps its own reference, so every release is a shared one.
        fn release_on_worker(self: Arc<Self>) -> Result<(), FsError> {
            self.releases.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
        fn list(&self, _: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
            unreachable!()
        }
        fn validate(&self) -> Result<(), FsError> {
            self.validations.fetch_add(1, Ordering::Relaxed);
            if self.changed.load(Ordering::Acquire) {
                Err(FsError::from_io(
                    "archive",
                    std::path::Path::new(""),
                    std::io::Error::other("changed"),
                ))
            } else {
                Ok(())
            }
        }
        fn read(
            &self,
            entry: u64,
            offset: u64,
            length: u32,
            _: &Cancellation,
        ) -> Result<Vec<u8>, FsError> {
            self.validate()?;
            let bytes = if entry == 2 { b"first" } else { b"other" };
            Ok(bytes[offset as usize..offset as usize + length as usize].to_vec())
        }
    }
    fn launch(mount: &Arc<Mount>, members: Vec<ArchiveNode>, focused: u64) -> ViewerLaunch {
        let entries: Arc<[Entry]> = members.into_iter().map(Entry::from_archive).collect();
        ViewerLaunch {
            find: None,
            associations: Default::default(),
            tree: None,
            generated: None,
            preferences: Arc::new(Default::default()),
            archive: Some(ArchiveViewer {
                mount: mount.clone(),
                source_version: vec![1, 2, 3],
                focused: VirtualEntryId {
                    mount: 8,
                    entry: focused,
                },
            }),
            view: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
            focused: PathBuf::new(),
            basic: true,
            disabled_plugins: Arc::from([]),
        }
    }

    fn member(entry: u64, display_name: String, size: Option<u64>) -> ArchiveNode {
        ArchiveNode {
            modified: None,
            id: VirtualEntryId { mount: 8, entry },
            parent: 1,
            raw_name: b"same.txt".as_slice().into(),
            display_name,
            kind: EntryKind::File,
            size,
            readable: true,
            access: Default::default(),
        }
    }

    #[test]
    fn viewer_reads_duplicate_named_members_by_id_and_retains_the_mount_lease() {
        let mount = Arc::new(Mount::default());
        let members = (2..=3)
            .map(|entry| member(entry, "same.txt".into(), Some(5)))
            .collect();
        let launch = launch(&mount, members, 2);
        let closed = Arc::new(AtomicBool::new(false));
        let order = prepare(&launch, &closed).unwrap();
        drop(launch);
        let first = order.focused.as_ref().unwrap();
        assert!(first.path().as_os_str().is_empty());
        assert_eq!(first.read_at(0, 5).unwrap(), b"first");
        let other = order.capture.as_ref().unwrap()(1, &closed).unwrap();
        assert_eq!(other.read_at(0, 5).unwrap(), b"other");
        mount.changed.store(true, Ordering::Release);
        assert!(first.read_at(0, 5).is_err());
        closed.store(true, Ordering::Release);
    }

    /// A member capture carries its own resource scope, as HTML inside an archive does;
    /// releasing it is confirmed and returns its mount reference exactly once.
    #[test]
    fn released_member_captures_return_their_mount_reference_once() {
        let mount = Arc::new(Mount::default());
        let members = (2..=3)
            .map(|entry| member(entry, "same.txt".into(), Some(5)))
            .collect();
        let launch = launch(&mount, members, 2);
        let closed = Arc::new(AtomicBool::new(false));
        let order = prepare(&launch, &closed).unwrap();
        drop(launch);
        let other = order.capture.as_ref().unwrap()(1, &closed).unwrap();
        assert!(other.resources().is_some());
        let first = order.focused.unwrap();
        first.release_on_worker().unwrap();
        assert_eq!(mount.releases.load(Ordering::Relaxed), 1);
        other.release_on_worker().unwrap();
        assert_eq!(mount.releases.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn large_archive_folders_keep_a_window_and_lease_members_only_when_opened() {
        let mount = Arc::new(Mount::default());
        let count = 3_000u64;
        // 2,000-byte captions: the complete order needs about 12 MiB of descriptors.
        let caption = |entry: u64| format!("{entry:0>2000}");
        let members = (2..count + 2)
            .map(|entry| {
                let size = (entry != 1_501).then_some(5);
                member(entry, caption(entry), size)
            })
            .collect();
        let launch = launch(&mount, members, 1_500);
        let closed = Arc::new(AtomicBool::new(false));
        let order = prepare(&launch, &closed).unwrap();
        let length = order.files.len();
        assert!(length > 1_000 && length < count as usize, "{length}");
        let first = 1_500 - order.current as u64;
        for (index, label) in order.files.iter().enumerate() {
            assert_eq!(label, &PathBuf::from(caption(first + index as u64)));
        }
        let middle = length / 2;
        assert!(
            order.current.abs_diff(middle) <= 1,
            "{} of {length}",
            order.current
        );
        assert!(order.focused.is_ok());
        let leases = mount.validations.load(Ordering::Relaxed);
        assert!(leases <= 2, "only the focused member is leased: {leases}");
        let capture = order.capture.as_ref().unwrap();
        assert_eq!(
            capture(order.current + 1, &closed).unwrap_err(),
            "Archive member size unavailable",
            "an unusable sibling fails when opened, not the whole launch"
        );
        assert!(capture(order.current - 1, &closed).is_ok());
        assert!(capture(length, &closed).is_err());
    }
}

#[cfg(test)]
mod tree_tests {
    use super::*;
    use midnight_core::*;
    #[derive(Debug)]
    struct Mount;
    impl ArchiveMount for Mount {
        fn id(&self) -> u64 {
            8
        }
        fn list(&self, _: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
            panic!("Tree normalization must not read a newly opened folder")
        }
        fn validate(&self) -> Result<(), FsError> {
            Ok(())
        }
        fn read(&self, entry: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
            Ok(vec![entry as u8])
        }
    }
    #[test]
    fn archive_tree_retains_member_identity_and_skips_unrelated_unreadable_files() {
        let entries: Arc<[Entry]> = (2..=4)
            .map(|entry| {
                Entry::from_archive(ArchiveNode {
                    modified: None,
                    id: VirtualEntryId { mount: 8, entry },
                    parent: 7,
                    raw_name: Arc::from([entry as u8, 255]),
                    display_name: "same.txt".into(),
                    kind: EntryKind::File,
                    size: Some(1),
                    readable: entry != 4,
                    access: Default::default(),
                })
            })
            .collect();
        let index = Arc::new(DirectoryIndex::from_entries(&entries));
        let rows = [2usize, 1, 0]
            .into_iter()
            .enumerate()
            .map(|(position, original)| {
                let mut entry = entries[original].clone();
                entry.id = EntryId(position as u64 + 20);
                TreeDisplayRow {
                    id: TreeRowId(entry.id.0),
                    entry,
                    parent: None,
                    depth: 0,
                    disclosure: TreeDisclosure::None,
                    expanded: false,
                    selectable: true,
                    navigable: false,
                    context_only: false,
                    error: None,
                    error_details: None,
                    original_root_entry: Some(entries[original].id),
                    prefix_end: 0,
                    search_match: false,
                }
            })
            .collect();
        let view = Arc::new(
            TreeProjection::new(
                TreeRootSnapshot {
                    pane: PaneId::Left,
                    navigation: NavigationId(1),
                    entries: entries.clone(),
                    index: index.clone(),
                },
                TreeViewStamp {
                    pane: PaneId::Left,
                    navigation: NavigationId(1),
                    session: TreeSessionId(1),
                    revision: TreeRevision(1),
                },
                TreeLayoutRevision(1),
                rows,
            )
            .unwrap(),
        );
        let mut launch = ViewerLaunch {
            find: None,
            associations: Default::default(),
            generated: None,
            preferences: Default::default(),
            tree: Some(TreeViewer {
                view,
                focused: TreeRowId(22),
            }),
            archive: Some(ArchiveViewer {
                mount: Arc::new(Mount),
                source_version: vec![19],
                focused: VirtualEntryId { mount: 8, entry: 2 },
            }),
            entries,
            view: index,
            focused: PathBuf::new(),
            basic: false,
            disabled_plugins: Arc::from([]),
        };
        let closed = Arc::new(AtomicBool::new(false));
        let capture: crate::NativeCapture =
            Arc::new(|_, _| panic!("archive must not invoke native capture"));
        let order = crate::capture::prepare_with(&launch, &closed, capture.clone()).unwrap();
        assert_eq!(order.files.len(), 2);
        assert_eq!(order.current, 1);
        let first = order.focused.as_ref().unwrap();
        assert_eq!(first.read_at(0, 1).unwrap(), [2]);
        assert!(first.path().as_os_str().is_empty());
        let other = order.capture.as_ref().unwrap()(0, &closed).unwrap();
        assert_eq!(other.read_at(0, 1).unwrap(), [3]);
        launch.tree.as_mut().unwrap().focused = TreeRowId(20);
        launch.archive.as_mut().unwrap().focused.entry = 4;
        assert!(crate::capture::prepare_with(&launch, &closed, capture).is_err());
    }
}
