//! Fake captured authorities and native adapters: no desktop UI or filesystem writes.
use super::*;
use std::{
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::ThreadId,
    time::Duration,
};

#[derive(Debug, Default)]
pub(crate) struct Probe {
    pub calls: Mutex<Vec<(&'static str, Vec<PathBuf>, ThreadId)>>,
    pub dropped: AtomicBool,
    pub drop_thread: Mutex<Option<ThreadId>>,
}
impl Probe {
    pub fn record(&self, action: &'static str, paths: &[PathBuf]) {
        self.calls
            .lock()
            .unwrap()
            .push((action, paths.to_vec(), std::thread::current().id()));
    }
    pub async fn wait_for_drop(&self) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !self.dropped.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_ne!(
            self.drop_thread.lock().unwrap().unwrap(),
            std::thread::current().id()
        );
    }
}
#[derive(Debug)]
pub(crate) struct Source {
    pub paths: Vec<PathBuf>,
    pub occurrences: Vec<TreeCopyOccurrence>,
    pub intent: FileClipboardIntent,
    pub rejected: bool,
    pub cancel_during_resolve: bool,
    pub probe: Arc<Probe>,
}
impl Source {
    pub fn new(paths: Vec<PathBuf>, probe: Arc<Probe>) -> Self {
        let occurrences = (0..paths.len())
            .map(|index| TreeCopyOccurrence {
                stamp: TreeViewStamp {
                    pane: PaneId::Left,
                    navigation: NavigationId(5),
                    session: TreeSessionId(3),
                    revision: TreeRevision(9),
                },
                row: TreeRowId(index as u64 + 1),
            })
            .collect();
        Self {
            paths,
            occurrences,
            intent: FileClipboardIntent::Copy,
            rejected: false,
            cancel_during_resolve: false,
            probe,
        }
    }
}
impl CapturedClipboardFiles for Source {
    fn intent(&self) -> FileClipboardIntent {
        FileClipboardIntent::Copy
    }
    fn tree_copy_occurrence(&self, root: usize) -> Option<TreeCopyOccurrence> {
        self.occurrences.get(root).copied()
    }
    fn resolve(&self, cancel: &Cancellation) -> Result<FileClipboard, FsError> {
        self.probe.record("resolve", &self.paths);
        if self.rejected {
            return Err(error(io::ErrorKind::PermissionDenied, "revoked capture"));
        }
        if self.cancel_during_resolve {
            cancel.cancel();
        }
        Ok(FileClipboard {
            paths: self.paths.clone(),
            intent: self.intent,
        })
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        *self.probe.drop_thread.lock().unwrap() = Some(std::thread::current().id());
        self.probe.dropped.store(true, Ordering::Release);
    }
}
#[derive(Default)]
pub(crate) struct Gate {
    entered: AtomicBool,
    released: Mutex<bool>,
    wake: Condvar,
}
impl Gate {
    fn block(&self) {
        self.entered.store(true, Ordering::Release);
        let mut released = self.released.lock().unwrap();
        while !*released {
            released = self.wake.wait(released).unwrap();
        }
    }
    pub async fn wait(&self) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !self.entered.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
}
pub(crate) struct Release(pub Arc<Gate>);
impl Drop for Release {
    fn drop(&mut self) {
        *self.0.released.lock().unwrap() = true;
        self.0.wake.notify_all();
    }
}
pub(crate) struct Native {
    pub probe: Arc<Probe>,
    pub gate: Option<Arc<Gate>>,
    pub properties_available: bool,
}
impl Native {
    pub fn new(probe: Arc<Probe>) -> Self {
        Self {
            probe,
            gate: None,
            properties_available: true,
        }
    }
    fn enter(&self, action: &'static str, paths: &[PathBuf]) {
        assert!(
            !self.probe.dropped.load(Ordering::Acquire),
            "source must outlive native call"
        );
        self.probe.record(action, paths);
        if let Some(gate) = &self.gate {
            gate.block();
        }
        assert!(
            !self.probe.dropped.load(Ordering::Acquire),
            "source must outlive native return"
        );
    }
}
impl OpenService for Native {
    fn open_entry(&self, path: &Path) -> Result<OpenEntryResult, FsError> {
        self.enter("open", &[path.to_owned()]);
        Ok(OpenEntryResult::File { process: None })
    }
}
impl PropertyService for Native {
    fn show_properties(&self, paths: &[PathBuf]) -> Result<bool, FsError> {
        self.enter("properties", paths);
        Ok(self.properties_available)
    }
}
impl ShellMenuService for Native {
    fn show_shell_menu(&self, request: &ShellMenuRequest) -> Result<ShellMenuOutcome, FsError> {
        self.enter("menu", &request.paths);
        Ok(ShellMenuOutcome::Dismissed)
    }
}
impl IconService for Native {
    fn load_icon(&self, _: &IconKey) -> Result<Option<IconPixels>, FsError> {
        Ok(None)
    }
}
impl TerminalService for Native {
    fn shell_flavor(&self) -> ShellFlavor {
        ShellFlavor::Posix
    }
    fn launch_terminal(&self, _: &TerminalRequest) -> Result<Box<dyn TerminalProcess>, FsError> {
        unreachable!()
    }
}
impl LocationService for Native {
    fn locations(&self) -> Result<Vec<NavigationLocation>, FsError> {
        Ok(Vec::new())
    }
}
impl DirectoryService for Native {
    fn read_directory(
        &self,
        _: &Path,
        _: &Cancellation,
        _: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        unreachable!()
    }
    fn entry_metadata(&self, path: &Path) -> Result<EntryMetadata, FsError> {
        self.enter("metadata", &[path.to_owned()]);
        Ok(EntryMetadata {
            size: Some(7),
            ..Default::default()
        })
    }
}
impl VolumeService for Native {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        unreachable!()
    }
}
impl StorageService for Native {
    fn space_info(&self, _: &Path) -> Result<SpaceInfo, FsError> {
        unreachable!()
    }
}

#[test]
fn invalid_capture_payloads_and_changed_intent_are_rejected_whole() {
    let root = std::env::temp_dir();
    for paths in [
        vec![],
        vec!["relative".into()],
        vec![root.join("bad\0name")],
        vec![root.join("same"); MAX_FILE_CLIPBOARD_PATHS + 1],
        vec![root.join("x".repeat(MAX_FILE_CLIPBOARD_BYTES + 1))],
    ] {
        let source = Source::new(paths, Arc::new(Probe::default()));
        assert!(resolve(&source, &Cancellation::default()).is_err());
    }
    let mut source = Source::new(vec![root.join("valid")], Arc::new(Probe::default()));
    source.intent = FileClipboardIntent::Move;
    assert_eq!(
        resolve(&source, &Cancellation::default()).unwrap_err().kind,
        io::ErrorKind::InvalidData
    );
}
