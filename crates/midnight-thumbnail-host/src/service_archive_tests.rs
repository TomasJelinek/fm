use super::*;
use crate::{archive_source::ArchiveThumbnailSource, providers::Manifest};
use midnight_core::{ArchiveMount, ArchiveNode, Cancellation, EntryKind, FsError, VirtualEntryId};
use std::{path::Path, sync::Condvar, time::Instant};

#[derive(Debug, Default)]
struct Gate {
    released: Mutex<bool>,
    ready: Condvar,
}
impl Gate {
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.ready.notify_all();
    }
}
#[derive(Debug)]
struct BlockingMount(Arc<Gate>);
impl ArchiveMount for BlockingMount {
    fn id(&self) -> u64 {
        8
    }
    fn list(&self, _: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        unreachable!()
    }
    fn validate(&self) -> Result<(), FsError> {
        Ok(())
    }
    fn persistent_member_identity(
        &self,
        entry: u64,
        expected: &[u8],
    ) -> Result<Option<Vec<u8>>, FsError> {
        assert_eq!(expected, [1]);
        Ok(Some(
            [b"stable archive fixture".as_slice(), &entry.to_le_bytes()].concat(),
        ))
    }
    fn read(
        &self,
        entry: u64,
        _: u64,
        length: u32,
        cancel: &Cancellation,
    ) -> Result<Vec<u8>, FsError> {
        assert!(!cancel.is_cancelled());
        if entry < 4 {
            let released = self.0.released.lock().unwrap();
            let (_released, timeout) = self
                .0
                .ready
                .wait_timeout_while(released, Duration::from_secs(3), |open| !*open)
                .unwrap();
            if timeout.timed_out() {
                return Err(FsError::from_io(
                    "fixture timeout",
                    Path::new(""),
                    io::ErrorKind::TimedOut.into(),
                ));
            }
        }
        assert!(
            !cancel.is_cancelled(),
            "viewport cancellation must not terminate the shared mount"
        );
        Ok(vec![42; length as usize])
    }
}
fn target(mount: Arc<BlockingMount>, token: u64) -> Target {
    Target {
        token,
        edge: 96,
        identity: b"archive-preview-v1".to_vec(),
        extension: "raw".into(),
        selected: false,
        hovered: false,
        capture: ArchiveThumbnailSource {
            mount,
            node: ArchiveNode {
                id: VirtualEntryId {
                    mount: 8,
                    entry: token + 1,
                },
                parent: 1,
                raw_name: Arc::from(b"same.raw".as_slice()),
                display_name: "same.raw".into(),
                kind: EntryKind::File,
                size: Some(1),
                readable: true,
                modified: None,
            },
            source_version: vec![1],
        }
        .capture(),
    }
}
struct Running {
    input: Arc<Mutex<Mailbox>>,
    gate: Arc<Gate>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Drop for Running {
    fn drop(&mut self) {
        self.input.lock().unwrap().stopped = true;
        self.gate.release();
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

#[test]
fn cancelled_shared_archive_reads_retain_both_real_worker_slots_until_they_drain() {
    let root = tempfile::tempdir().unwrap();
    let cache = Arc::new(Mutex::new(
        Cache::open(&root.path().join("cache"), 1_000_000, 0).unwrap(),
    ));
    let gate = Arc::new(Gate::default());
    let mount = Arc::new(BlockingMount(gate.clone()));
    let input = Arc::new(Mutex::new(Mailbox {
        next: Some(View {
            generation: 1,
            targets: vec![target(mount.clone(), 1), target(mount.clone(), 2)],
        }),
        retries: HashMap::new(),
        stopped: false,
        clear_cache: false,
        cache_location: None,
        notice: None,
    }));
    let (started_tx, started) = mpsc::sync_channel(4);
    let (drained_tx, drained) = mpsc::sync_channel(4);
    let renderer: Renderer = Arc::new(move |target, _, work, _, cancel| {
        let source = (target.capture)(cancel)?;
        started_tx.send(work.token).unwrap();
        source.read_at(0, 1)?;
        drained_tx
            .send((work.token, cancel.load(Ordering::Acquire)))
            .unwrap();
        // Even a native call that returns successful bytes after cancellation cannot publish
        // stale results: the real service controller still checks generation and visibility.
        Ok(CachedFrame {
            edge: 96,
            frame: Arc::new(Frame {
                id: work.token,
                frame: 0,
                width: 1,
                height: 1,
                rgba: vec![42; 4],
            }),
        })
    });
    let provider = Provider {
        manifest: Manifest {
            protocol: 1,
            id: "fixture".into(),
            version: "1".into(),
            executable: "unused".into(),
            extensions: vec!["raw".into()],
            video: false,
        },
        executable: PathBuf::new(),
    };
    let (output, ready) = mpsc::sync_channel(4);
    let worker_input = input.clone();
    let worker = thread::spawn(move || run(worker_input, output, vec![provider], cache, renderer));
    let _running = Running {
        input: input.clone(),
        gate: gate.clone(),
        worker: Some(worker),
    };
    let mut first = [
        started.recv_timeout(Duration::from_secs(3)).unwrap(),
        started.recv_timeout(Duration::from_secs(3)).unwrap(),
    ];
    first.sort_unstable();
    assert_eq!(first, [1, 2]);
    input.lock().unwrap().next = Some(View {
        generation: 2,
        targets: vec![target(mount.clone(), 3)],
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    while input.lock().unwrap().next.is_some() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(1));
    }
    assert!(matches!(
        started.recv_timeout(Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    assert!(ready.try_recv().is_err());
    gate.release();
    let mut old = [
        drained.recv_timeout(Duration::from_secs(3)).unwrap(),
        drained.recv_timeout(Duration::from_secs(3)).unwrap(),
    ];
    old.sort_unstable();
    assert_eq!(old, [(1, true), (2, true)]);
    assert_eq!(started.recv_timeout(Duration::from_secs(3)).unwrap(), 3);
    let result = ready.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!((result.generation, result.token), (2, 3));
    assert!(mount.validate().is_ok());
    assert_eq!(mount.read(4, 0, 1, &Cancellation::default()).unwrap(), [42]);
}
