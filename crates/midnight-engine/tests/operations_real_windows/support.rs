//! Real disk fixtures deliberately retained under the repository QA directory.
use midnight_core::{
    Command, CopyChoice, CopyConflict, CopyDecision, CopyEvent, CopyFailure, CopyOperationState,
    CopyProgress, CopyRequest, Event, FileClipboard, FileClipboardIntent, OperationId,
    TransferVerification,
};
use midnight_engine::Engine;
use midnight_platform::LocalFileSystem;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{sync::mpsc, task::JoinHandle};

pub struct Fixture {
    pub root: PathBuf,
}

impl Fixture {
    pub fn new(label: &str) -> Self {
        let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/qa/operations-find");
        fs::create_dir_all(&parent).unwrap();
        let root = tempfile::Builder::new()
            .prefix(&format!("transfer-real-{label}-"))
            .tempdir_in(parent)
            .unwrap()
            .keep();
        let root = fs::canonicalize(root).unwrap();
        eprintln!("REAL OPERATION FIXTURE: {}", root.display());
        Self { root }
    }

    pub fn directory(&self, name: &str) -> PathBuf {
        let path = self.root.join(name);
        fs::create_dir_all(&path).unwrap();
        path
    }

    pub fn record(&self, line: &str) {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.root.join("EVIDENCE.txt"))
            .unwrap();
        writeln!(file, "{line}").unwrap();
        eprintln!("{line}");
    }
}

/// Independent disk enumeration and SHA-256, including empty directories.
pub fn manifest(root: &Path) -> BTreeMap<PathBuf, Option<(u64, [u8; 32])>> {
    fn visit(root: &Path, path: &Path, result: &mut BTreeMap<PathBuf, Option<(u64, [u8; 32])>>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let relative = path.strip_prefix(root).unwrap().to_owned();
            let metadata = fs::symlink_metadata(&path).unwrap();
            assert!(
                !metadata.file_type().is_symlink(),
                "fixture must not escape"
            );
            if metadata.is_dir() {
                result.insert(relative, None);
                visit(root, &path, result);
            } else {
                let mut file = fs::File::open(&path).unwrap();
                let mut hash = Sha256::new();
                let mut buffer = [0; 65_536];
                let mut length = 0;
                loop {
                    let count = file.read(&mut buffer).unwrap();
                    if count == 0 {
                        break;
                    }
                    hash.update(&buffer[..count]);
                    length += count as u64;
                }
                result.insert(relative, Some((length, hash.finalize().into())));
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}

pub fn populate(directory: &Path) -> (u64, u64) {
    fs::create_dir_all(directory.join("empty directory")).unwrap();
    fs::create_dir_all(directory.join("nested/深い/🦀")).unwrap();
    let mut long = directory.to_owned();
    for _ in 0..7 {
        long.push("long-Český-中文-component-0123456789");
    }
    fs::create_dir_all(&long).unwrap();
    fs::write(long.join("long.txt"), b"long path contents").unwrap();
    for (name, bytes) in [
        ("empty.bin", &b""[..]),
        ("Žluťoučký 中文 🧪.txt", &b"Unicode filename"[..]),
        ("e\u{301}.txt", &b"decomposed filename"[..]),
        ("shell & $ ; [literal].txt", &b"literal filename"[..]),
        ("nested/深い/🦀/leaf.bin", &b"nested leaf"[..]),
        ("readonly.txt", &b"read only source"[..]),
    ] {
        fs::write(directory.join(name), bytes).unwrap();
    }
    for index in 0..257 {
        let bytes: Vec<u8> = (0..(index * 17 + 1))
            .map(|offset| (offset * 31 + index * 7) as u8)
            .collect();
        fs::write(directory.join(format!("item-{index:03}.dat")), bytes).unwrap();
    }
    for size in [
        65_535, 65_536, 196_608, 196_609, 262_143, 262_144, 8_388_625,
    ] {
        let bytes: Vec<u8> = (0..size)
            .map(|index| (index * 13 + index / 997) as u8)
            .collect();
        fs::write(directory.join(format!("boundary-{size}.bin")), bytes).unwrap();
    }
    let path = directory.join("readonly.txt");
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(path, permissions).unwrap();
    let entries = manifest(directory);
    let files = entries.values().flatten().count() as u64;
    let bytes = entries.values().flatten().map(|(length, _)| length).sum();
    (files, bytes)
}

pub struct Harness {
    pub commands: mpsc::Sender<Command>,
    pub terminal_order: Vec<u64>,
    events: mpsc::Receiver<Event>,
    task: Option<JoinHandle<()>>,
    terminals: HashMap<u64, Arc<CopyProgress>>,
}

impl Harness {
    pub fn new() -> Self {
        let provider = Arc::new(LocalFileSystem::new(None));
        let (commands, receiver) = mpsc::channel(128);
        let (sender, events) = mpsc::channel(512);
        let engine = Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            provider.clone(),
            sender,
        )
        .with_transfers(provider.clone())
        .with_mutations(provider);
        Self {
            commands,
            terminal_order: Vec::new(),
            events,
            task: Some(tokio::spawn(engine.run(receiver))),
            terminals: HashMap::new(),
        }
    }

    pub async fn enqueue(
        &self,
        id: u64,
        sources: &[PathBuf],
        destination: &Path,
        verification: TransferVerification,
        moving: bool,
    ) {
        let request = CopyRequest {
            verification,
            id: OperationId(id),
            clipboard: FileClipboard {
                paths: sources.into(),
                intent: if moving {
                    FileClipboardIntent::Move
                } else {
                    FileClipboardIntent::Copy
                },
            },
            destination: destination.into(),
        };
        self.commands
            .send(if moving {
                Command::EnqueueMove(request)
            } else {
                Command::EnqueueCopy(request)
            })
            .await
            .unwrap();
    }

    async fn next(&mut self, deadline: tokio::time::Instant) -> CopyEvent {
        tokio::time::timeout_at(deadline, async {
            loop {
                if let Event::Copy(event) = self.events.recv().await.expect("engine channel closed")
                {
                    if let CopyEvent::Progress(progress) = &event
                        && progress.state.is_terminal()
                    {
                        self.terminal_order.push(progress.id.0);
                        self.terminals.insert(progress.id.0, progress.clone());
                    }
                    return event;
                }
            }
        })
        .await
        .expect("operation stalled for 120 seconds")
    }

    pub async fn decide(&self, id: u64, version: u64, choice: CopyChoice) {
        self.commands
            .send(Command::DecideCopy(CopyDecision {
                id: OperationId(id),
                version,
                choice,
            }))
            .await
            .unwrap();
    }

    pub async fn conflict(&mut self, id: u64) -> Arc<CopyConflict> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
        loop {
            match self.next(deadline).await {
                CopyEvent::Conflict(conflict) if conflict.id == OperationId(id) => return conflict,
                CopyEvent::Failure(error) => panic!("unexpected operation failure: {error:?}"),
                CopyEvent::Progress(progress) if progress.id == OperationId(id) => {
                    assert!(
                        !progress.state.is_terminal(),
                        "expected conflict: {progress:?}"
                    );
                }
                _ => {}
            }
        }
    }

    pub async fn failure(&mut self, id: u64) -> Arc<CopyFailure> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
        loop {
            match self.next(deadline).await {
                CopyEvent::Failure(failure) if failure.id == OperationId(id) => return failure,
                CopyEvent::Progress(progress) if progress.id == OperationId(id) => {
                    assert!(
                        !progress.state.is_terminal(),
                        "expected failure: {progress:?}"
                    );
                }
                other => assert!(!matches!(other, CopyEvent::Conflict(_)), "{other:?}"),
            }
        }
    }

    pub async fn terminal(&mut self, id: u64) -> Arc<CopyProgress> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
        loop {
            if let Some(progress) = self.terminals.remove(&id) {
                return progress;
            }
            match self.next(deadline).await {
                CopyEvent::Failure(error) => panic!("unexpected operation failure: {error:?}"),
                CopyEvent::Conflict(conflict) => panic!("unexpected conflict: {conflict:?}"),
                _ => {}
            }
        }
    }

    pub async fn cancel_requested(&mut self, id: u64) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            if let CopyEvent::Progress(progress) = self.next(deadline).await
                && progress.id == OperationId(id)
                && progress.state == CopyOperationState::CancelRequested
            {
                return;
            }
        }
    }

    pub async fn completed(&mut self, id: u64) -> Arc<CopyProgress> {
        let progress = self.terminal(id).await;
        assert_eq!(
            progress.state,
            CopyOperationState::Completed,
            "{progress:?}"
        );
        assert_eq!(progress.failed, 0, "{progress:?}");
        assert!(progress.errors.is_empty(), "{progress:?}");
        progress
    }

    pub async fn close(mut self) {
        self.commands.send(Command::Shutdown).await.unwrap();
        self.events.close();
        tokio::time::timeout(Duration::from_secs(30), self.task.take().unwrap())
            .await
            .unwrap()
            .unwrap();
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        // Panicking assertions must not leave an Engine blocked at a user decision.
        self.events.close();
        let _ = self.commands.try_send(Command::Shutdown);
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}
