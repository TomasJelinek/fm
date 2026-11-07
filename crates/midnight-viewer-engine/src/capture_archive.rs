//! Archive members are content leases; native paths are never reconstructed.
use midnight_core::{ArchiveMount, Cancellation, ViewerLaunch};
use midnight_plugin_host::{CapturedFile, ContentSource, Error};
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
    length: u64,
    name: String,
    version: Vec<u8>,
    closed: Arc<AtomicBool>,
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
            length: node.size.ok_or("Archive member size unavailable")?,
            name: node.display_name.clone(),
            version: archive.source_version.clone(),
            closed: closed.clone(),
        });
        // This value is a format-selection label only, never opened as a native path.
        files.push((PathBuf::from(&node.display_name), None));
        sources.push(source);
    }
    files[current].1 =
        Some(CapturedFile::from_source(sources[current].clone()).map_err(|e| e.to_string()));
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
                let result =
                    CapturedFile::from_source(sources[index].clone()).map_err(|e| e.to_string());
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
