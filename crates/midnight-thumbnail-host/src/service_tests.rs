use super::*;
use crate::providers::Manifest;

struct Bytes;
impl Source for Bytes {
    fn length(&self) -> u64 {
        0
    }
    fn version(&self) -> Vec<u8> {
        vec![1]
    }
    fn validate(&self) -> io::Result<()> {
        Ok(())
    }
    fn read_at(&self, _: u64, _: u32) -> io::Result<Vec<u8>> {
        Ok(vec![])
    }
}
fn key(identity: &[u8]) -> Key {
    Key::new("fixture", identity, &Bytes, (96, 96), "v1", 0).unwrap()
}
fn target(token: u64) -> Target {
    Target {
        token,
        identity: vec![token as u8],
        extension: "raw".into(),
        selected: false,
        hovered: false,
        capture: Arc::new(|| Ok(Arc::new(Bytes))),
    }
}

#[test]
fn every_animation_sample_retains_the_same_captured_source_revision() {
    struct Revision(u8);
    impl Source for Revision {
        fn length(&self) -> u64 {
            1
        }
        fn version(&self) -> Vec<u8> {
            vec![self.0]
        }
        fn validate(&self) -> io::Result<()> {
            Ok(())
        }
        fn read_at(&self, _: u64, _: u32) -> io::Result<Vec<u8>> {
            Ok(vec![self.0])
        }
    }
    let captures = Arc::new(std::sync::atomic::AtomicU8::new(1));
    let count = captures.clone();
    let capture = pin_capture(Arc::new(move || {
        Ok(Arc::new(Revision(count.fetch_add(1, Ordering::SeqCst))))
    }));
    for _ in 0..6 {
        let source = capture().unwrap();
        assert_eq!(source.version(), [1]);
        assert_eq!(source.read_at(0, 1).unwrap(), [1]);
    }
    assert_eq!(
        captures.load(Ordering::SeqCst),
        2,
        "Only the first sample may capture the visible token"
    );
}

#[test]
fn clearing_waits_for_old_decode_and_only_new_generation_may_repopulate() {
    let parent = std::env::var_os("MIDNIGHT_NATIVE_TEST_ROOT")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("CARGO_TARGET_DIR")
                .map(|path| PathBuf::from(path).join("private-tests"))
        })
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(2)
                .expect("thumbnail host belongs to the repository crates directory")
                .join("_build/private-tests")
        });
    std::fs::create_dir_all(&parent).unwrap();
    let root = tempfile::tempdir_in(parent).unwrap();
    let cache = Arc::new(Mutex::new(
        Cache::open(&root.path().join("cache"), 1024 * 1024, 0).unwrap(),
    ));
    let input = Arc::new(Mutex::new(Mailbox {
        next: None,
        stopped: false,
        clear_cache: false,
        notice: None,
    }));
    let (ready_tx, ready) = mpsc::sync_channel(4);
    let (started_tx, started) = mpsc::sync_channel(1);
    let render: Renderer = Arc::new(move |target, _, work, cache, cancel| {
        if work.generation == 1 {
            started_tx.send(()).unwrap();
            while !cancel.load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(1));
            }
        }
        // Simulate a decoder reaching persistence just as cancellation arrives. The controller
        // must drain it before clear, rather than assuming cancellation already stopped the write.
        cache.lock().unwrap().put(
            &key(&target.identity),
            &Bytes,
            Frame {
                id: work.token,
                width: 1,
                height: 1,
                frame: 0,
                rgba: vec![work.generation as u8, 0, 0, 255],
            },
        )
    });
    let worker_input = input.clone();
    let worker_cache = cache.clone();
    let worker = thread::spawn(move || {
        run(
            worker_input,
            ready_tx,
            vec![Provider {
                executable: PathBuf::new(),
                manifest: Manifest {
                    protocol: 1,
                    id: "fixture".into(),
                    version: "v1".into(),
                    executable: "fixture".into(),
                    extensions: vec!["raw".into()],
                    video: false,
                },
            }],
            worker_cache,
            render,
        )
    });
    input.lock().unwrap().next = Some(View {
        generation: 1,
        targets: vec![target(1)],
    });
    started.recv_timeout(Duration::from_secs(3)).unwrap();
    {
        let mut mailbox = input.lock().unwrap();
        mailbox.clear_cache = true;
        mailbox.next = Some(View {
            generation: 2,
            targets: vec![target(2)],
        });
    }
    let result = ready.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!(
        (result.generation, result.token, result.frame.rgba[0]),
        (2, 2, 2)
    );
    input.lock().unwrap().stopped = true;
    worker.join().unwrap();
    let mut cache = cache.lock().unwrap();
    assert!(
        cache.get(&key(&[1]), &Bytes).unwrap().is_none(),
        "Old in-flight write was removed after it actually finished"
    );
    assert_eq!(cache.get(&key(&[2]), &Bytes).unwrap().unwrap().rgba[0], 2);
    assert!(
        ready.try_recv().is_err(),
        "No stale frame reached presentation"
    );
}
