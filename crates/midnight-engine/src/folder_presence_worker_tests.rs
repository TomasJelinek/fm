use super::*;
use std::sync::mpsc as blocking;

struct Blocked {
    files: Arc<Files>,
    entered: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    release: Mutex<blocking::Receiver<()>>,
}
impl DirectoryService for Blocked {
    fn read_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        if let Some(entered) = self.entered.lock().unwrap().take() {
            entered.send(()).unwrap();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
        }
        self.files.read_directory(path, cancel, emit)
    }
    fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
        panic!("presence must not request entry metadata")
    }
}
impl VolumeService for Blocked {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        panic!("presence must not query volumes")
    }
}
impl StorageService for Blocked {
    fn space_info(&self, _: &Path) -> Result<SpaceInfo, FsError> {
        panic!("presence must not query space")
    }
}

#[tokio::test]
async fn blocked_presence_keeps_only_latest_viewport_and_cancels_obsolete_receipts() {
    let (original, snapshots, files) = fixture();
    let (entered, waiting) = tokio::sync::oneshot::channel();
    let (release, released) = blocking::sync_channel(1);
    let filesystem = Arc::new(Blocked {
        files: files.clone(),
        entered: Mutex::new(Some(entered)),
        release: Mutex::new(released),
    });
    let (sender, mut events) = tokio::sync::mpsc::channel(8);
    let mut tasks = JoinSet::new();
    let controller = Controller::new(
        TaskLifetime::testing(filesystem.clone()),
        crate::ReportArena::testing(),
        filesystem,
        snapshots,
        std::array::from_fn(|_| Arc::new(Semaphore::new(1))),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
    );
    let request = |revision, id| {
        Arc::new(FolderPresenceRequest {
            root: original.root.clone(),
            revision: FolderPresenceRevision(revision),
            visible: vec![EntryId(id)],
            cancel: Cancellation::default(),
        })
    };
    let first = request(1, 1);
    controller.submit(first.clone()).await;
    tokio::time::timeout(Duration::from_secs(3), waiting)
        .await
        .unwrap()
        .unwrap();
    let second = request(2, 2);
    controller.submit(second.clone()).await;
    let latest = request(3, 3);
    controller.submit(latest.clone()).await;
    assert!(first.cancel.is_cancelled());
    assert!(second.cancel.is_cancelled());
    release.send(()).unwrap();
    let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap();
    let Event::FolderPresence(snapshot) = event else {
        panic!("unexpected receipt")
    };
    assert_eq!(snapshot.revision, latest.revision);
    assert_eq!(
        &*snapshot.values,
        &[(EntryId(3), FolderPresence::HasFolders)]
    );
    assert_eq!(files.visited.lock().unwrap().len(), 2);
    assert!(events.try_recv().is_err());
    drop(controller);
    tokio::time::timeout(Duration::from_secs(3), async {
        while tasks.join_next().await.is_some() {}
    })
    .await
    .unwrap();
}
