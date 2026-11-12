use super::*;
use crate::{
    providers::NativeProvider,
    tree_actor::{Session, WorkerBudget},
    tree_projection::ProjectionOptions,
};
use midnight_core::{DirectoryIndex, TreeRootSnapshot};
use std::sync::{Condvar, mpsc as sync_mpsc};

pub(super) struct Gate {
    released: Mutex<bool>,
    wake: Condvar,
}
impl Gate {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            released: Mutex::new(false),
            wake: Condvar::new(),
        })
    }
    pub(super) fn wait(&self) {
        let mut released = self.released.lock().unwrap();
        while !*released {
            released = self.wake.wait(released).unwrap();
        }
    }
    pub(super) fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.wake.notify_all();
    }
}
pub(super) struct Release(pub(super) Arc<Gate>);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.release();
    }
}
pub(super) struct Probe {
    pub(super) caller: ThreadId,
    pub(super) entered: sync_mpsc::Sender<ThreadId>,
    pub(super) gate: Arc<Gate>,
}
impl Drop for Probe {
    fn drop(&mut self) {
        let current = thread::current().id();
        let _ = self.entered.send(current);
        // A broken caller-side Drop must report RED rather than deadlock the test.
        if current != self.caller {
            self.gate.wait();
        }
    }
}
#[derive(Default)]
pub(super) struct LifetimeFiles {
    pub(super) _probe: Option<Probe>,
    pub(super) panic_on_read: bool,
}
impl DirectoryService for LifetimeFiles {
    fn read_directory(
        &self,
        _: &Path,
        _: &Cancellation,
        _: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        assert!(
            !self.panic_on_read,
            "watch lifetime fixture native probe panic"
        );
        Ok(())
    }
    fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
        Ok(EntryMetadata::default())
    }
}
impl VolumeService for LifetimeFiles {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        unreachable!()
    }
}
impl StorageService for LifetimeFiles {
    fn space_info(&self, _: &Path) -> Result<SpaceInfo, FsError> {
        unreachable!()
    }
}
pub(super) fn captured_scope(files: Arc<dyn FileSystem>) -> tree::TreeScope {
    let path = std::env::current_dir()
        .unwrap()
        .join("_build/watch-lifetime-root");
    let root = TreeRootSnapshot {
        pane: PaneId::Left,
        navigation: NavigationId(4),
        index: Arc::new(DirectoryIndex::from_entries(&[])),
        entries: Arc::default(),
    };
    let native = NativeProvider::new(files);
    let cancel = Cancellation::default();
    let opened = native.capture_root(&path, &root.entries, &cancel).unwrap();
    let mut session = Session::new(
        root,
        opened,
        Arc::default(),
        WorkerBudget::new(Arc::new(tokio::sync::Semaphore::new(1))),
        ProjectionOptions::default(),
        &cancel,
    )
    .unwrap();
    let view = session.projection().clone();
    assert!(session.commit(&view));
    let scope = session.watch_scope().unwrap().unwrap();
    assert_eq!(
        scope.targets.len(),
        1,
        "fixture must retain a real mounted root"
    );
    scope
}
pub(super) struct AffineNative(pub(super) sync_mpsc::Sender<(ThreadId, ThreadId)>);
struct Subscription {
    created: ThreadId,
    dropped: sync_mpsc::Sender<(ThreadId, ThreadId)>,
}
impl Drop for Subscription {
    fn drop(&mut self) {
        let _ = self.dropped.send((self.created, thread::current().id()));
    }
}
impl WatchSubscription for Subscription {}
impl WatchService for AffineNative {
    fn watch_directory(
        &self,
        _: &Path,
        _: ChangeCallback,
    ) -> Result<Box<dyn WatchSubscription>, FsError> {
        unreachable!()
    }
    fn watch_directories(
        &self,
        _: &[PathBuf],
        _: Arc<dyn Fn(u32) + Send + Sync>,
    ) -> Result<Box<dyn WatchSubscription>, FsError> {
        Ok(Box::new(Subscription {
            created: thread::current().id(),
            dropped: self.0.clone(),
        }))
    }
}
