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
        panic!("cached thumbnail tests must not read source content")
    }
}
fn key(identity: &[u8]) -> Key {
    Key::new("fixture", identity, &Bytes, (96, 96), "v1", 0).unwrap()
}
fn target(token: u64) -> Target {
    Target {
        token,
        edge: 96,
        identity: vec![token as u8],
        extension: "raw".into(),
        selected: false,
        hovered: false,
        capture: Arc::new(|_| Ok(Arc::new(Bytes))),
    }
}

#[test]
fn a_larger_cached_decode_covers_smaller_requests_even_when_the_source_image_is_tiny() {
    let parent = std::env::var_os("MIDNIGHT_NATIVE_TEST_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(2)
                .unwrap()
                .join("_build/private-tests")
        });
    std::fs::create_dir_all(&parent).unwrap();
    let root = tempfile::tempdir_in(parent).unwrap();
    let target = target(77);
    let provider = Provider {
        manifest: Manifest {
            protocol: 1,
            id: "fixture".into(),
            version: "v1".into(),
            executable: "decoder-deliberately-absent".into(),
            extensions: vec!["raw".into()],
            video: false,
        },
        executable: root.path().join("decoder-deliberately-absent"),
    };
    let identity = blake3::hash(&target.identity);
    let key = Key::new("fixture", identity.as_bytes(), &Bytes, (256, 256), "v1", 0).unwrap();
    let expected = Frame {
        id: 0,
        width: 1,
        height: 1,
        frame: 0,
        rgba: vec![21, 34, 55, 255],
    };
    let cache_root = root.path().join("cache");
    let mut cache = Cache::open(&cache_root, 1024 * 1024, 1024).unwrap();
    cache.put(&key, &Bytes, expected.clone()).unwrap();
    drop(cache);
    let cache = Mutex::new(Cache::open(&cache_root, 1024 * 1024, 1024).unwrap());
    for _ in 0..2 {
        let work = Work {
            generation: 1,
            token: 77,
            edge: 96,
            frame: 0,
        };
        let result = render(&target, &provider, work, &cache, &AtomicBool::new(false)).unwrap();
        assert_eq!(*result.frame, expected);
        assert_eq!(
            result.edge, 256,
            "coverage is retained independently of the tiny image dimensions"
        );
    }
    assert!(
        cache.lock().unwrap().get(&key, &Bytes).unwrap().is_some(),
        "reusing a larger record must retain it"
    );
}

#[test]
fn reopened_disk_frame_bypasses_decoder_and_content_reads_but_not_source_revision() {
    struct MetadataOnly(u8);
    impl Source for MetadataOnly {
        fn length(&self) -> u64 {
            17
        }
        fn version(&self) -> Vec<u8> {
            vec![self.0]
        }
        fn validate(&self) -> io::Result<()> {
            Ok(())
        }
        fn read_at(&self, _: u64, _: u32) -> io::Result<Vec<u8>> {
            panic!("a valid disk hit must not read/decode source content")
        }
    }
    let parent = std::env::var_os("MIDNIGHT_NATIVE_TEST_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(2)
                .expect("thumbnail host is in repository crates")
                .join("_build/private-tests")
        });
    std::fs::create_dir_all(&parent).unwrap();
    let root = tempfile::tempdir_in(parent).unwrap();
    let mut provider = Provider {
        manifest: Manifest {
            protocol: 1,
            id: "fixture".into(),
            version: "v1".into(),
            executable: "decoder-deliberately-absent".into(),
            extensions: vec!["raw".into()],
            video: false,
        },
        executable: root.path().join("decoder-deliberately-absent"),
    };
    let target = Target {
        capture: Arc::new(|_| Ok(Arc::new(MetadataOnly(9)))),
        ..target(77)
    };
    let identity = blake3::hash(&target.identity);
    let key = Key::new(
        "fixture",
        identity.as_bytes(),
        &MetadataOnly(9),
        (96, 96),
        "v1",
        0,
    )
    .unwrap();
    let expected = Frame {
        // Persistent pixels are independent of the visible scheduling token.
        id: 0,
        width: 1,
        height: 1,
        frame: 0,
        rgba: vec![21, 34, 55, 255],
    };
    let cache_root = root.path().join("cache");
    let mut cache = Cache::open(&cache_root, 1024 * 1024, 0).unwrap();
    cache.put(&key, &MetadataOnly(9), expected.clone()).unwrap();
    drop(cache);
    let cache = Mutex::new(Cache::open(&cache_root, 1024 * 1024, 0).unwrap());
    let work = Work {
        generation: 1,
        token: 77,
        edge: 96,
        frame: 0,
    };
    let cancel = AtomicBool::new(false);
    assert_eq!(
        *render(&target, &provider, work, &cache, &cancel)
            .unwrap()
            .frame,
        expected
    );
    let stop = Arc::new(AtomicBool::new(false));
    let capturing = stop.clone();
    let interrupted = Target {
        capture: Arc::new(move |_| {
            capturing.store(true, Ordering::Release);
            Ok(Arc::new(MetadataOnly(9)))
        }),
        ..target.clone()
    };
    assert!(
        matches!(render(&interrupted, &provider, work, &cache, &stop),
            Err(error) if error.kind() == io::ErrorKind::Interrupted),
        "cancellation during capture must not publish even an already cached frame"
    );
    provider.manifest.version = "v2".into();
    assert!(
        render(&target, &provider, work, &cache, &cancel).is_err(),
        "decoder-version change must miss the old disk frame and report unavailable decoder"
    );
    provider.manifest.version = "v1".into();
    let changed = Target {
        capture: Arc::new(|_| Ok(Arc::new(MetadataOnly(10)))),
        ..target
    };
    assert!(
        render(&changed, &provider, work, &cache, &cancel).is_err(),
        "fresh source revision must never reuse old disk pixels"
    );
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
    let capture = pin_capture(Arc::new(move |_| {
        Ok(Arc::new(Revision(count.fetch_add(1, Ordering::SeqCst))))
    }));
    for _ in 0..6 {
        let source = capture(&AtomicBool::new(false)).unwrap();
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
fn a_rejected_delivery_retries_from_cache_after_the_view_changes_without_recapturing() {
    let parent = std::env::var_os("MIDNIGHT_NATIVE_TEST_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(2)
                .unwrap()
                .join("_build/private-tests")
        });
    std::fs::create_dir_all(&parent).unwrap();
    let root = tempfile::tempdir_in(parent).unwrap();
    let mut cache = Cache::open(&root.path().join("cache"), 1024 * 1024, 0).unwrap();
    let captures = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = captures.clone();
    let capture: Capture = Arc::new(move |_| {
        count.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::new(Bytes))
    });
    let targets: Vec<_> = [1, 2]
        .into_iter()
        .map(|token| Target {
            edge: 256,
            capture: capture.clone(),
            ..target(token)
        })
        .collect();
    for target in &targets {
        let identity = blake3::hash(&target.identity);
        let key = Key::new("fixture", identity.as_bytes(), &Bytes, (256, 256), "v1", 0).unwrap();
        cache
            .put(
                &key,
                &Bytes,
                Frame {
                    id: 0,
                    width: 1,
                    height: 1,
                    frame: 0,
                    rgba: vec![target.token as u8; 4],
                },
            )
            .unwrap();
    }
    let input = Arc::new(Mutex::new(Mailbox {
        next: None,
        retries: HashMap::new(),
        stopped: false,
        clear_cache: false,
        notice: None,
    }));
    let (output, ready) = mpsc::sync_channel(4);
    let worker_input = input.clone();
    let provider = Provider {
        manifest: Manifest {
            protocol: 1,
            id: "fixture".into(),
            version: "v1".into(),
            executable: "decoder-deliberately-absent".into(),
            extensions: vec!["raw".into()],
            video: false,
        },
        executable: root.path().join("decoder-deliberately-absent"),
    };
    let worker = thread::spawn(move || {
        run(
            worker_input,
            output,
            vec![provider],
            Arc::new(Mutex::new(cache)),
            Arc::new(render),
        );
    });
    let mut service = Service {
        mailbox: input,
        ready,
        thread: Some(worker),
    };
    assert!(service.set_view(View {
        generation: 1,
        targets: targets.clone()
    }));
    let first = service.ready.recv_timeout(Duration::from_secs(3)).unwrap();
    let second = service.ready.recv_timeout(Duration::from_secs(3)).unwrap();
    let rejected = if first.token == 1 { first } else { second };
    assert_eq!(rejected.edge, 256);
    assert!(matches!(
        service.ready.recv_timeout(Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    assert!(service.set_view(View {
        generation: 1,
        targets: vec![targets[0].clone()]
    }));
    let work = Work {
        generation: rejected.generation,
        token: rejected.token,
        edge: rejected.edge,
        frame: rejected.frame.frame,
    };
    assert!(service.retry_delivery(work, 96));
    let retried = service.ready.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!(
        (retried.token, retried.edge, retried.frame.rgba.as_slice()),
        (1, 256, &[1, 1, 1, 1][..])
    );
    assert_eq!(
        captures.load(Ordering::SeqCst),
        2,
        "retry keeps the visible token's immutable source capture"
    );
    assert!(matches!(
        service.ready.recv_timeout(Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    service.stop();
    service.thread.take().unwrap().join().unwrap();
}

#[test]
fn increasing_one_visible_edge_reuses_its_pinned_capture_and_leaves_other_cached_frames() {
    let parent = std::env::var_os("MIDNIGHT_NATIVE_TEST_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(2)
                .unwrap()
                .join("_build/private-tests")
        });
    std::fs::create_dir_all(&parent).unwrap();
    let root = tempfile::tempdir_in(parent).unwrap();
    let cache = Arc::new(Mutex::new(
        Cache::open(&root.path().join("cache"), 1024 * 1024, 0).unwrap(),
    ));
    let input = Arc::new(Mutex::new(Mailbox {
        next: None,
        retries: HashMap::new(),
        stopped: false,
        clear_cache: false,
        notice: None,
    }));
    let (output, ready) = mpsc::sync_channel(4);
    let captures = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = captures.clone();
    let capture: Capture = Arc::new(move |_| {
        count.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::new(Bytes))
    });
    let targets = |edge| {
        vec![
            Target {
                edge,
                capture: capture.clone(),
                ..target(1)
            },
            Target {
                capture: capture.clone(),
                ..target(2)
            },
        ]
    };
    let render: Renderer = Arc::new(|target, _, work, cache, cancel| {
        let source = (target.capture)(cancel)?;
        let key = Key::new(
            "fixture",
            &target.identity,
            &*source,
            (work.edge, work.edge),
            "v1",
            0,
        )?;
        let frame = cache.lock().unwrap().put(
            &key,
            &*source,
            Frame {
                id: 0,
                width: 1,
                height: 1,
                frame: 0,
                rgba: vec![work.token as u8; 4],
            },
        )?;
        Ok(CachedFrame {
            frame,
            edge: work.edge,
        })
    });
    let worker_input = input.clone();
    let worker_cache = cache.clone();
    let worker = thread::spawn(move || {
        run(
            worker_input,
            output,
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
        targets: targets(96),
    });
    for _ in 0..2 {
        assert_eq!(ready.recv_timeout(Duration::from_secs(3)).unwrap().edge, 96);
    }
    input.lock().unwrap().next = Some(View {
        generation: 1,
        targets: targets(256),
    });
    let upgrade = ready.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!((upgrade.token, upgrade.edge), (1, 256));
    input.lock().unwrap().stopped = true;
    worker.join().unwrap();
    assert_eq!(
        captures.load(Ordering::SeqCst),
        2,
        "resolution changes must not recapture visible source identity"
    );
    assert!(
        ready.try_recv().is_err(),
        "only the insufficient thumbnail needs another publication"
    );
    let mut cache = cache.lock().unwrap();
    for (token, edge) in [(1, 96), (1, 256), (2, 96)] {
        let key = Key::new("fixture", &[token], &Bytes, (edge, edge), "v1", 0).unwrap();
        assert!(
            cache.get(&key, &Bytes).unwrap().is_some(),
            "upgrading one thumbnail never clears other resolutions or identities"
        );
    }
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
        retries: HashMap::new(),
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
        cache
            .lock()
            .unwrap()
            .put(
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
            .map(|frame| CachedFrame {
                frame,
                edge: work.edge,
            })
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
