use midnight_core::*;
use midnight_engine::Engine;
use midnight_platform::LocalFileSystem;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;

struct BlockedRoot {
    entered: Arc<AtomicBool>,
    released: Arc<AtomicBool>,
}
impl FindService for BlockedRoot {
    fn open_find_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn FindSource>, FsError> {
        self.entered.store(true, Ordering::SeqCst);
        while !self.released.load(Ordering::SeqCst) && !cancel.is_cancelled() {
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(Box::new(EmptyRoot(FindEntry {
            path: path.to_owned(),
            kind: EntryKind::Directory,
            metadata: EntryMetadata::default(),
            identity: TransferIdentity {
                device: 1,
                object: [1; 16],
            },
            revision: FindRevision([0; 6]),
        })))
    }
}
struct EmptyRoot(FindEntry);
impl FindSource for EmptyRoot {
    fn entry(&self) -> &FindEntry {
        &self.0
    }
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        Ok(())
    }
    fn next_child(&mut self, _: &Cancellation) -> Result<Option<FindChild>, FsError> {
        Ok(None)
    }
    fn read_chunk(&mut self, _: &mut [u8], _: &Cancellation) -> Result<usize, FsError> {
        Ok(0)
    }
}

#[tokio::test]
async fn current_directory_arrives_while_root_open_is_still_blocked() {
    let temporary = tempfile::tempdir().unwrap();
    let path: PathBuf = temporary.path().to_owned();
    let entered = Arc::new(AtomicBool::new(false));
    let released = Arc::new(AtomicBool::new(false));
    let service = Arc::new(BlockedRoot {
        entered: entered.clone(),
        released: released.clone(),
    });
    let (send, commands) = mpsc::channel(16);
    let (events, mut receive) = mpsc::channel(16);
    let engine = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            Arc::new(LocalFileSystem::new(None)),
            events,
        )
        .with_find(service)
        .run(commands),
    );
    send.send(Command::StartFind(Box::new(FindRequest::new(
        FindSessionId(1),
        FindGeneration(1),
        vec![path.clone()],
    ))))
    .await
    .unwrap();
    let observed = tokio::time::timeout(Duration::from_millis(1200), async {
        while let Some(event) = receive.recv().await {
            if let Event::Find(event) = event
                && let FindUpdate::CurrentDirectory(directory) = event.update
            {
                return Some(directory);
            }
        }
        None
    })
    .await
    .ok()
    .flatten();
    let still_blocked = entered.load(Ordering::SeqCst) && !released.load(Ordering::SeqCst);
    released.store(true, Ordering::SeqCst);
    drop(send);
    tokio::time::timeout(Duration::from_secs(5), engine)
        .await
        .unwrap()
        .unwrap();
    assert!(still_blocked);
    assert_eq!(
        observed.as_deref(),
        Some(&path),
        "status must not wait for blocking native open"
    );
}

struct NestedService {
    root: PathBuf,
    read: bool,
    entered: Arc<AtomicBool>,
    released: Arc<AtomicBool>,
}
struct NestedSource {
    entry: FindEntry,
    level: u8,
    consumed: bool,
    read: bool,
    entered: Arc<AtomicBool>,
    released: Arc<AtomicBool>,
}
impl NestedService {
    fn source(&self, path: PathBuf, level: u8) -> NestedSource {
        NestedSource {
            entry: FindEntry {
                path,
                kind: if level == 2 {
                    EntryKind::File
                } else {
                    EntryKind::Directory
                },
                metadata: EntryMetadata {
                    size: (level == 2).then_some(6),
                    ..Default::default()
                },
                identity: TransferIdentity {
                    device: 1,
                    object: [level; 16],
                },
                revision: FindRevision([0; 6]),
            },
            level,
            consumed: false,
            read: self.read,
            entered: self.entered.clone(),
            released: self.released.clone(),
        }
    }
}
impl FindService for NestedService {
    fn open_find_source(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Box<dyn FindSource>, FsError> {
        assert_eq!(path, self.root);
        Ok(Box::new(self.source(path.to_owned(), 0)))
    }
}
impl NestedSource {
    fn wait(&self, cancel: &Cancellation) {
        self.entered.store(true, Ordering::SeqCst);
        while !self.released.load(Ordering::SeqCst) && !cancel.is_cancelled() {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
impl FindSource for NestedSource {
    fn entry(&self) -> &FindEntry {
        &self.entry
    }
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        Ok(())
    }
    fn next_child(&mut self, cancel: &Cancellation) -> Result<Option<FindChild>, FsError> {
        if self.consumed {
            return Ok(None);
        }
        self.consumed = true;
        if self.level == 1 && !self.read {
            self.wait(cancel);
        }
        let service = NestedService {
            root: self.entry.path.clone(),
            read: self.read,
            entered: self.entered.clone(),
            released: self.released.clone(),
        };
        Ok(Some(FindChild::Entry(Box::new(service.source(
            self.entry.path.join(if self.level == 0 {
                "nested 文档"
            } else {
                "content.txt"
            }),
            self.level + 1,
        )))))
    }
    fn read_chunk(&mut self, buffer: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        assert_eq!(self.level, 2);
        if self.consumed {
            return Ok(0);
        }
        self.consumed = true;
        if self.read {
            self.wait(cancel);
        }
        buffer[..6].copy_from_slice(b"needle");
        Ok(6)
    }
}
async fn nested_status(read: bool) {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().to_owned();
    let entered = Arc::new(AtomicBool::new(false));
    let released = Arc::new(AtomicBool::new(false));
    let service = Arc::new(NestedService {
        root: root.clone(),
        read,
        entered: entered.clone(),
        released: released.clone(),
    });
    let (send, commands) = mpsc::channel(16);
    let (events, mut receive) = mpsc::channel(16);
    let engine = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            Arc::new(LocalFileSystem::new(None)),
            events,
        )
        .with_find(service)
        .run(commands),
    );
    let mut request = FindRequest::new(FindSessionId(1), FindGeneration(1), vec![root.clone()]);
    request.content = Some(FindContentQuery {
        text: "needle".into(),
        case_sensitive: true,
        whole_words: false,
    });
    send.send(Command::StartFind(Box::new(request)))
        .await
        .unwrap();
    let observed = tokio::time::timeout(Duration::from_millis(1200), async {
        while let Some(event) = receive.recv().await {
            if let Event::Find(event) = event
                && let FindUpdate::CurrentDirectory(path) = event.update
            {
                return Some(path);
            }
        }
        None
    })
    .await
    .ok()
    .flatten();
    let blocked = entered.load(Ordering::SeqCst) && !released.load(Ordering::SeqCst);
    released.store(true, Ordering::SeqCst);
    drop(send);
    tokio::time::timeout(Duration::from_secs(5), engine)
        .await
        .unwrap()
        .unwrap();
    assert!(blocked);
    assert_eq!(observed.as_deref(), Some(&root.join("nested 文档")));
}
#[tokio::test]
async fn current_directory_tracks_nested_enumeration_before_it_unblocks() {
    nested_status(false).await;
}
#[tokio::test]
async fn current_directory_tracks_parent_during_blocked_content_read() {
    nested_status(true).await;
}
