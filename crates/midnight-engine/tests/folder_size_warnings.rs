//! Actual command routing and cancelled-worker report delivery for both native scan actions.
use midnight_core::*;
use midnight_engine::Engine;
use std::{
    io,
    path::Path,
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};
use tokio::sync::{Notify, mpsc};

#[derive(Default)]
struct Files {
    entered: Notify,
    released: Mutex<bool>,
    wake: Condvar,
}
impl Files {
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.wake.notify_all();
    }
}
impl DirectoryService for Files {
    fn read_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        match path.to_str().unwrap() {
            "root" => {
                emit(Entry::new(
                    EntryId(1),
                    "root/folder".into(),
                    EntryKind::Directory,
                ));
            }
            "root/folder" => {
                if emit(Entry::new(
                    EntryId(2),
                    "outside 資料".into(),
                    EntryKind::File,
                )) {
                    emit(Entry::new(
                        EntryId(3),
                        "root/folder/wait".into(),
                        EntryKind::Directory,
                    ));
                }
            }
            "root/folder/wait" => {
                self.entered.notify_one();
                let mut released = self.released.lock().unwrap();
                while !*released {
                    let (next, timeout) = self
                        .wake
                        .wait_timeout(released, Duration::from_secs(10))
                        .unwrap();
                    released = next;
                    if timeout.timed_out() {
                        return Err(FsError::from_io(
                            "test gate",
                            path,
                            io::ErrorKind::TimedOut.into(),
                        ));
                    }
                }
                if cancel.is_cancelled() {
                    return Err(FsError::from_io(
                        "cancelled enumeration",
                        path,
                        io::ErrorKind::Interrupted.into(),
                    ));
                }
            }
            _ => {
                return Err(FsError::from_io(
                    "enumerate",
                    path,
                    io::ErrorKind::NotFound.into(),
                ));
            }
        }
        Ok(())
    }
    fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
        Ok(EntryMetadata::default())
    }
}
impl VolumeService for Files {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        Ok(Vec::new())
    }
}
impl StorageService for Files {
    fn storage_kind(&self, _: &Path) -> Result<StorageKind, FsError> {
        Ok(StorageKind::LocalSsd)
    }
    fn space_info(&self, _: &Path) -> Result<SpaceInfo, FsError> {
        Ok(SpaceInfo {
            total: 100,
            available: 100,
        })
    }
}
struct Release(Arc<Files>);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.release();
    }
}

async fn next(events: &mut mpsc::Receiver<Event>) -> Event {
    tokio::time::timeout(Duration::from_secs(8), events.recv())
        .await
        .expect("responsive report route")
        .unwrap()
}

async fn cancellation_report(single: bool) {
    let files = Arc::new(Files::default());
    let _release = Release(files.clone());
    let (commands, input) = mpsc::channel(16);
    let (output, mut events) = mpsc::channel(32);
    let engine = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            files.clone(),
            output,
        )
        .run(input),
    );
    commands
        .send(Command::Navigate {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            path: "root".into(),
            sort: SortSpec::default(),
        })
        .await
        .unwrap();
    let root = loop {
        if let Event::ListingFinished { entries, .. } = next(&mut events).await {
            break entries[0].id;
        }
    };
    let scan = FolderSizeScanId(9);
    let command = if single {
        Command::ComputeEntrySize {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            scan,
            entry: root,
        }
    } else {
        Command::ComputeFolderSizes {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            scan,
            selection: None,
        }
    };
    commands.send(command).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), files.entered.notified())
        .await
        .unwrap();
    commands
        .send(Command::CancelFolderSizes {
            pane: PaneId::Left,
            scan,
        })
        .await
        .unwrap();
    loop {
        if let Event::FolderSizesFinished {
            scan: actual,
            cancelled,
            ..
        } = next(&mut events).await
        {
            assert_eq!(actual, scan);
            assert!(cancelled);
            break;
        }
    }
    // The OS call returns after the responsive cancellation acknowledgment.
    files.release();
    let size = loop {
        if let Event::FolderSizeUpdated {
            scan: actual, size, ..
        } = next(&mut events).await
        {
            assert_eq!(actual, scan);
            if size.warnings.is_some() {
                break size;
            }
        }
    };
    assert!(!size.finished);
    assert_eq!(size.errors, 1);
    let report = size.warnings.unwrap();
    assert_eq!(report.summary().observed, 1);
    commands
        .send(Command::OpenSizeWarnings {
            request: DesktopRequestId(91),
            report,
            cancel: Cancellation::default(),
        })
        .await
        .unwrap();
    let source = loop {
        if let Event::SizeWarningsReady { request, result } = next(&mut events).await {
            assert_eq!(request, DesktopRequestId(91));
            break result.unwrap();
        }
    };
    let text = tokio::task::spawn_blocking(move || {
        let mut bytes = Vec::new();
        let mut chunk = [0; 1024];
        loop {
            let count = source.read_at(bytes.len() as u64, &mut chunk).unwrap();
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..count]);
        }
        String::from_utf8(bytes).unwrap()
    })
    .await
    .unwrap();
    assert!(text.contains("outside 資料"));
    assert!(text.contains("Scan cancelled"));
    assert!(text.contains("outside the immediate directory"));
    assert!(!text.contains("Kind: Interrupted"));
    commands.send(Command::Shutdown).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), engine)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_scan_preserves_complete_report_after_cancel_acknowledgment() {
    cancellation_report(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn individual_scan_preserves_complete_report_after_cancel_acknowledgment() {
    cancellation_report(true).await;
}
