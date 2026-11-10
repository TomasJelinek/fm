//! Archive members are content leases; native paths are never reconstructed.
use midnight_core::{ArchiveMount, Cancellation, EntryKind, ViewerLaunch};
use midnight_plugin_host::{CapturedFile, ContentResources, ContentSource, Error};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

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
}
pub(crate) fn prepare(
    launch: &ViewerLaunch,
    closed: &Arc<AtomicBool>,
) -> Result<super::capture::Order, String> {
    let archive = launch.archive.as_ref().ok_or("Missing archive focus")?;
    let mut nodes = Vec::new();
    let mut bytes = 0usize;
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
        bytes = bytes
            .saturating_add(node.display_name.len().saturating_mul(8))
            .saturating_add(512);
        if bytes > 8 * 1024 * 1024 {
            return Err("The captured file order exceeds the 8 MiB viewer budget.".into());
        }
        nodes.push((*row, node));
    }
    nodes.sort_unstable_by_key(|(row, _)| *row);
    let current = nodes
        .iter()
        .position(|(_, node)| node.id == archive.focused)
        .ok_or("Archive member is unavailable")?;
    let mut files = Vec::with_capacity(nodes.len());
    let mut sources = Vec::with_capacity(nodes.len());
    for (_, node) in nodes {
        let source = Arc::new(Member {
            mount: archive.mount.clone(),
            entry: node.id.entry,
            parent: node.parent,
            length: node.size.ok_or("Archive member size unavailable")?,
            name: node.display_name.clone(),
            version: archive.source_version.clone(),
            closed: closed.clone(),
        });
        // This value is a format-selection label only, never opened as a native path.
        files.push((PathBuf::from(&node.display_name), None));
        sources.push(source);
    }
    files[current].1 = Some(capture_member(sources[current].clone()).map_err(|e| e.to_string()));
    let (sender, ready) = mpsc::sync_channel(16);
    let closed = closed.clone();
    tokio::runtime::Handle::try_current()
        .map_err(|e| e.to_string())?
        .spawn_blocking(move || {
            for step in 1..sources.len() {
                if closed.load(Ordering::Acquire) {
                    return;
                }
                let index = (current + step) % sources.len();
                let result = capture_member(sources[index].clone()).map_err(|e| e.to_string());
                let mut event = (index, result);
                loop {
                    if closed.load(Ordering::Acquire) {
                        return;
                    }
                    match sender.try_send(event) {
                        Ok(()) => break,
                        Err(mpsc::TrySendError::Full(value)) => event = value,
                        Err(mpsc::TrySendError::Disconnected(_)) => return,
                    }
                    std::thread::park_timeout(Duration::from_millis(8));
                }
            }
        });
    Ok(super::capture::Order {
        files,
        current,
        ready,
    })
}

#[cfg(test)]
#[path = "archive_resources_tests.rs"]
mod resource_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use midnight_core::*;
    #[derive(Debug, Default)]
    struct Mount {
        changed: AtomicBool,
    }
    impl ArchiveMount for Mount {
        fn id(&self) -> u64 {
            8
        }
        fn list(&self, _: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
            unreachable!()
        }
        fn validate(&self) -> Result<(), FsError> {
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
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn viewer_reads_duplicate_named_members_by_id_and_retains_the_mount_lease() {
        let mount = Arc::new(Mount::default());
        let entries: Arc<[Entry]> = (2..=3)
            .map(|entry| {
                Entry::from_archive(ArchiveNode {
                    modified: None,
                    id: VirtualEntryId { mount: 8, entry },
                    parent: 1,
                    raw_name: b"same.txt".as_slice().into(),
                    display_name: "same.txt".into(),
                    kind: EntryKind::File,
                    size: Some(5),
                    readable: true,
                })
            })
            .collect::<Vec<_>>()
            .into();
        let launch = ViewerLaunch {
            associations: Default::default(),
            tree: None,
            generated: None,
            preferences: Arc::new(Default::default()),
            archive: Some(ArchiveViewer {
                mount: mount.clone(),
                source_version: vec![1, 2, 3],
                focused: VirtualEntryId { mount: 8, entry: 2 },
            }),
            view: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
            focused: PathBuf::new(),
            basic: true,
            disabled_plugins: Arc::from([]),
        };
        let closed = Arc::new(AtomicBool::new(false));
        let order = prepare(&launch, &closed).unwrap();
        drop(launch);
        let first = order.files[order.current]
            .1
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap();
        assert!(first.path().as_os_str().is_empty());
        assert_eq!(first.read_at(0, 5).unwrap(), b"first");
        let (_, other) = order.ready.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(other.unwrap().read_at(0, 5).unwrap(), b"other");
        mount.changed.store(true, Ordering::Release);
        assert!(first.read_at(0, 5).is_err());
        closed.store(true, Ordering::Release);
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
    #[tokio::test]
    async fn archive_tree_retains_member_identity_and_skips_unrelated_unreadable_files() {
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
        let first = order.files[1].1.as_ref().unwrap().as_ref().unwrap();
        assert_eq!(first.read_at(0, 1).unwrap(), [2]);
        assert!(first.path().as_os_str().is_empty());
        let (next, other) = order.ready.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(next, 0);
        assert_eq!(other.unwrap().read_at(0, 1).unwrap(), [3]);
        launch.tree.as_mut().unwrap().focused = TreeRowId(20);
        launch.archive.as_mut().unwrap().focused.entry = 4;
        assert!(crate::capture::prepare_with(&launch, &closed, capture).is_err());
    }
}
