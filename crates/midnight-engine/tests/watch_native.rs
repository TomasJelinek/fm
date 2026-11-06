use midnight_core::*;
use midnight_engine::Engine;
use midnight_platform::LocalFileSystem;
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Notify, mpsc};

struct TrackedWatch {
    filesystem: Arc<LocalFileSystem>,
    registered: AtomicUsize,
    ready: Notify,
}

impl WatchService for TrackedWatch {
    fn watch_directory(
        &self,
        path: &Path,
        changed: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Box<dyn WatchSubscription>, FsError> {
        let guard = self.filesystem.watch_directory(path, changed)?;
        self.registered.fetch_add(1, Ordering::Release);
        self.ready.notify_one();
        Ok(guard)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepted_panes_receive_real_native_change_hints_and_can_reload() {
    let temporary = tempfile::tempdir().unwrap();
    let left = temporary.path().join("left");
    let right = temporary.path().join("right");
    std::fs::create_dir(&left).unwrap();
    std::fs::create_dir(&right).unwrap();
    let filesystem = Arc::new(LocalFileSystem::new(Some(temporary.path().into())));
    let watcher = Arc::new(TrackedWatch {
        filesystem: filesystem.clone(),
        registered: AtomicUsize::new(0),
        ready: Notify::new(),
    });
    let (commands, receiver) = mpsc::channel(16);
    let (sender, mut events) = mpsc::channel(32);
    let engine = tokio::spawn(
        Engine::new(filesystem, sender)
            .with_watch(watcher.clone())
            .run(receiver),
    );
    for (pane, path) in [(PaneId::Left, &left), (PaneId::Right, &right)] {
        commands
            .send(Command::Navigate {
                pane,
                navigation: NavigationId(1),
                path: path.clone(),
                sort: SortSpec::default(),
            })
            .await
            .unwrap();
    }
    let mut accepted = 0;
    while accepted < 2 {
        let event = tokio::time::timeout(Duration::from_secs(8), events.recv())
            .await
            .unwrap()
            .unwrap();
        if let Event::ListingFinished {
            pane,
            navigation,
            entries,
            ..
        } = event
        {
            let path = if pane == PaneId::Left {
                left.clone()
            } else {
                right.clone()
            };
            commands
                .send(Command::PaneCommitted(VisibleDirectory {
                    pane,
                    navigation,
                    path,
                    entries,
                }))
                .await
                .unwrap();
            accepted += 1;
        }
    }
    tokio::time::timeout(Duration::from_secs(8), async {
        while watcher.registered.load(Ordering::Acquire) < 2 {
            watcher.ready.notified().await;
        }
    })
    .await
    .expect("Both native watchers must register");
    std::fs::write(left.join("Žluťoučký-文档-👩🏽‍💻.txt"), b"new data").unwrap();
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if let Event::DirectoryChanged { pane, navigation } = events.recv().await.unwrap() {
                assert_eq!((pane, navigation), (PaneId::Left, NavigationId(1)));
                break;
            }
        }
    })
    .await
    .expect("Native event must survive debounce and reach the accepted pane");
    commands
        .send(Command::Navigate {
            pane: PaneId::Left,
            navigation: NavigationId(2),
            path: left.clone(),
            sort: SortSpec::default(),
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if let Event::ListingFinished {
                pane: PaneId::Left,
                navigation: NavigationId(2),
                entries,
                ..
            } = events.recv().await.unwrap()
            {
                assert!(
                    entries
                        .iter()
                        .any(|entry| entry.path == left.join("Žluťoučký-文档-👩🏽‍💻.txt")
                            && entry.metadata.size == Some(8))
                );
                break;
            }
        }
    })
    .await
    .unwrap();
    commands.send(Command::Shutdown).await.unwrap();
    drop(events);
    engine.await.unwrap();
}
