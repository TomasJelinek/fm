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
        confirmed_ssd: false,
        capture: Arc::new(|_| Ok(Arc::new(Bytes))),
    }
}
/// Private cache stores need an owner-only ancestry, so every test uses the same
/// repository build location unless CI supplies its native test root.
fn private_root() -> tempfile::TempDir {
    let parent = std::env::var_os("MIDNIGHT_NATIVE_TEST_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(2)
                .expect("thumbnail host belongs to the repository crates directory")
                .join("_build/private-tests")
        });
    std::fs::create_dir_all(&parent).unwrap();
    tempfile::tempdir_in(parent).unwrap()
}
fn fixture_provider(executable: PathBuf) -> Provider {
    Provider {
        manifest: Manifest {
            protocol: 1,
            id: "fixture".into(),
            version: "v1".into(),
            executable: "decoder-deliberately-absent".into(),
            extensions: vec!["raw".into()],
            video: false,
        },
        executable,
    }
}
/// Run the backend loop on its own thread; dropping the guard stops and joins it.
struct Backend {
    shared: Arc<Shared>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Backend {
    fn start(output: mpsc::SyncSender<Ready>, cache: Arc<Mutex<Cache>>, render: Renderer) -> Self {
        let shared = Shared::new();
        let input = shared.clone();
        let worker = thread::spawn(move || {
            run(
                input,
                output,
                vec![fixture_provider(PathBuf::new())],
                cache,
                render,
            )
        });
        Self {
            shared,
            worker: Some(worker),
        }
    }
    fn view(&self, generation: u64, targets: Vec<Target>) {
        self.shared.signal(|mailbox| {
            mailbox.next = Some(View {
                thumbnailer_associations: Default::default(),
                generation,
                targets,
            });
        });
    }
    fn wait_processed(&self, count: u64) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.shared.lock().processed < count {
            assert!(
                Instant::now() < deadline,
                "backend did not process completions"
            );
            thread::yield_now();
        }
    }
    fn stop(&mut self) {
        self.shared.signal(|mailbox| mailbox.stopped = true);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}
impl Drop for Backend {
    fn drop(&mut self) {
        if !thread::panicking() {
            self.stop();
        } else {
            self.shared.signal(|mailbox| mailbox.stopped = true);
        }
    }
}

#[test]
fn confirmed_ssd_capture_shares_the_io_gate_and_unknown_storage_stays_exclusive() {
    let root = private_root();
    let cache = Mutex::new(Cache::open(&root.path().join("cache"), 1024 * 1024, 0).unwrap());
    let provider = fixture_provider(root.path().join("decoder-deliberately-absent"));
    let work = Work {
        generation: 1,
        token: 7,
        edge: 96,
        frame: 0,
    };
    for confirmed_ssd in [true, false] {
        let observed = Arc::new(Mutex::new(None));
        let gate = observed.clone();
        let target = Target {
            confirmed_ssd,
            capture: Arc::new(move |_| {
                *gate.lock().unwrap() = Some(midnight_work_budget::shared_io().snapshot());
                Err(io::Error::other("admission observed"))
            }),
            ..target(7)
        };
        assert!(render(&target, &provider, work, &cache, &AtomicBool::new(false)).is_err());
        let snapshot = observed.lock().unwrap().take().unwrap();
        if confirmed_ssd {
            // A long transfer holding shared slots must not block warm SSD thumbnails.
            assert!(!snapshot.exclusive && snapshot.active_shared >= 1);
        } else {
            assert!(
                snapshot.exclusive,
                "unknown storage keeps exclusive capture"
            );
        }
    }
}

#[test]
fn an_interruption_nobody_requested_backs_off_instead_of_respawning_the_job() {
    let root = private_root();
    let cache = Arc::new(Mutex::new(
        Cache::open(&root.path().join("cache"), 1024 * 1024, 0).unwrap(),
    ));
    let (calls_tx, calls) = mpsc::channel();
    let render: Renderer = Arc::new(move |_, _, work, _, _| {
        calls_tx.send(work.token).unwrap();
        if work.token == 1 {
            // A stopped archive mount reports Interrupted for every member read.
            Err(io::ErrorKind::Interrupted.into())
        } else {
            Err(io::Error::other("unsupported fixture"))
        }
    });
    let (output, _ready) = mpsc::sync_channel(4);
    let mut backend = Backend::start(output, cache, render);
    backend.view(1, vec![target(1)]);
    assert_eq!(calls.recv_timeout(Duration::from_secs(5)).unwrap(), 1);
    backend.wait_processed(1);
    // Revisit the same generation: only the new row may start while row 1 backs off.
    backend.view(1, vec![target(1), target(2)]);
    assert_eq!(calls.recv_timeout(Duration::from_secs(5)).unwrap(), 2);
    backend.stop();
    assert_eq!(
        calls.try_iter().collect::<Vec<_>>(),
        Vec::<u64>::new(),
        "an uncancelled interruption must not respawn its decoder immediately"
    );
}

#[test]
fn saturated_presentation_still_cancels_stale_work_and_delivers_after_polling() {
    let root = private_root();
    let cache = Arc::new(Mutex::new(
        Cache::open(&root.path().join("cache"), 1024 * 1024, 0).unwrap(),
    ));
    let (release_tx, release) = mpsc::channel::<()>();
    let release = Mutex::new(release);
    let (events_tx, events) = mpsc::channel();
    let render: Renderer = Arc::new(move |_, _, work, _, cancel| {
        match work.token {
            2 if work.generation == 1 => release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap(),
            3 => {
                events_tx.send("3 started").unwrap();
                let deadline = Instant::now() + Duration::from_secs(5);
                while !cancel.load(Ordering::Acquire) && Instant::now() < deadline {
                    thread::yield_now();
                }
                events_tx
                    .send(if cancel.load(Ordering::Acquire) {
                        "3 cancelled"
                    } else {
                        "3 timed out"
                    })
                    .unwrap();
                return Err(io::ErrorKind::Interrupted.into());
            }
            _ => {}
        }
        Ok(CachedFrame {
            edge: work.edge,
            frame: Arc::new(Frame {
                id: 0,
                width: 1,
                height: 1,
                frame: 0,
                rgba: vec![work.token as u8; 4],
            }),
        })
    });
    let (output, ready) = mpsc::sync_channel(1);
    let mut backend = Backend::start(output, cache, render);
    // Like Service::poll, taking a frame frees a slot and signals the backend.
    let poll = |backend: &Backend| {
        let frame = ready.recv_timeout(Duration::from_secs(5)).unwrap();
        backend.shared.signal(|_| {});
        frame
    };
    let notified = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = notified.clone();
    backend.shared.lock().ready_notifier = Some(Arc::new(move || {
        count.fetch_add(1, Ordering::SeqCst);
    }));
    backend.view(1, vec![target(1), target(2), target(3)]);
    // Row 1 fills the one-slot queue; row 3 starts in its freed worker slot.
    assert_eq!(
        events.recv_timeout(Duration::from_secs(5)).unwrap(),
        "3 started"
    );
    release_tx.send(()).unwrap();
    backend.wait_processed(2);
    // Row 2 now waits for presentation. A new view must still cancel stale row 3.
    backend.view(2, vec![target(2)]);
    assert_eq!(
        events.recv_timeout(Duration::from_secs(5)).unwrap(),
        "3 cancelled"
    );
    let first = poll(&backend);
    assert_eq!((first.generation, first.token), (1, 1));
    // Generation 1 left the view, so its stashed row-2 frame is never presented; row 2 is
    // rendered again for generation 2 and delivered once presentation has room.
    let second = poll(&backend);
    assert_eq!((second.generation, second.token), (2, 2));
    backend.stop();
    assert!(ready.try_recv().is_err());
    assert_eq!(notified.load(Ordering::SeqCst), 2);
}

#[test]
fn a_larger_cached_decode_covers_smaller_requests_even_when_the_source_image_is_tiny() {
    let root = private_root();
    let target = target(77);
    let provider = fixture_provider(root.path().join("decoder-deliberately-absent"));
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
    let root = private_root();
    let mut provider = fixture_provider(root.path().join("decoder-deliberately-absent"));
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
    let root = private_root();
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
    let shared = Shared::new();
    let (output, ready) = mpsc::sync_channel(4);
    let worker_input = shared.clone();
    let provider = fixture_provider(root.path().join("decoder-deliberately-absent"));
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
        shared,
        ready,
        thread: Some(worker),
    };
    assert!(service.set_view(View {
        thumbnailer_associations: Default::default(),
        generation: 1,
        targets: targets.clone()
    }));
    let first = service.ready.recv_timeout(Duration::from_secs(3)).unwrap();
    let second = service.ready.recv_timeout(Duration::from_secs(3)).unwrap();
    let rejected = if first.token == 1 { first } else { second };
    assert_eq!(rejected.edge, 256);
    assert!(service.set_view(View {
        thumbnailer_associations: Default::default(),
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
    service.stop();
    service.thread.take().unwrap().join().unwrap();
    // Every delivery has happened once the backend has exited: nothing else was sent.
    assert!(service.ready.try_recv().is_err());
}

#[test]
fn increasing_one_visible_edge_reuses_its_pinned_capture_and_leaves_other_cached_frames() {
    let root = private_root();
    let cache = Arc::new(Mutex::new(
        Cache::open(&root.path().join("cache"), 1024 * 1024, 0).unwrap(),
    ));
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
    let mut backend = Backend::start(output, cache.clone(), render);
    backend.view(1, targets(96));
    for _ in 0..2 {
        assert_eq!(ready.recv_timeout(Duration::from_secs(3)).unwrap().edge, 96);
    }
    backend.view(1, targets(256));
    let upgrade = ready.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!((upgrade.token, upgrade.edge), (1, 256));
    backend.stop();
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
    let root = private_root();
    let cache = Arc::new(Mutex::new(
        Cache::open(&root.path().join("cache"), 1024 * 1024, 0).unwrap(),
    ));
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
    let mut backend = Backend::start(ready_tx, cache.clone(), render);
    backend.view(1, vec![target(1)]);
    started.recv_timeout(Duration::from_secs(3)).unwrap();
    backend.shared.signal(|mailbox| {
        mailbox.clear_cache = true;
        mailbox.next = Some(View {
            thumbnailer_associations: Default::default(),
            generation: 2,
            targets: vec![target(2)],
        });
    });
    let result = ready.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!(
        (result.generation, result.token, result.frame.rgba[0]),
        (2, 2, 2)
    );
    backend.stop();
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

#[test]
fn a_helper_that_cannot_load_its_libraries_is_reported_once() {
    let root = private_root();
    let cache = Arc::new(Mutex::new(
        Cache::open(&root.path().join("cache"), 1024 * 1024, 0).unwrap(),
    ));
    let render: Renderer = Arc::new(|_, _, _, _, _| {
        Err(io::Error::other(crate::decode::HelperCannotStart {
            helper: PathBuf::from("midnight-thumbnail-video.exe"),
            status: midnight_worker_platform::ExitStatus::Code(0xC000_0135),
            library: Some(midnight_worker_platform::UnloadableLibrary {
                path: PathBuf::from("avcodec-63.dll"),
                missing: false,
            }),
        }))
    });
    let (output, _ready) = mpsc::sync_channel(4);
    let mut backend = Backend::start(output, cache, render);
    backend.view(1, vec![target(1), target(2)]);
    backend.wait_processed(2);
    assert_eq!(
        backend.shared.lock().notice.take().as_deref(),
        Some(
            "Some previews are unavailable: the thumbnail helper midnight-thumbnail-video.exe \
             could not load its libraries (access denied): avcodec-63.dll"
        )
    );
    // Later previews of the same kind fail the same way without repeating the notice.
    backend.view(2, vec![target(3)]);
    backend.wait_processed(3);
    assert_eq!(backend.shared.lock().notice.take(), None);
    backend.stop();
}

/// A captured source double whose file is found changed after its capture (`changed`).
struct Captured {
    changed: Arc<AtomicBool>,
}
impl Source for Captured {
    fn length(&self) -> u64 {
        1
    }
    fn version(&self) -> Vec<u8> {
        vec![1]
    }
    fn validate(&self) -> io::Result<()> {
        if self.changed.load(Ordering::Acquire) {
            Err(io::Error::other("Native file changed after capture"))
        } else {
            Ok(())
        }
    }
    fn read_at(&self, _: u64, _: u32) -> io::Result<Vec<u8>> {
        unreachable!("rendering doubles read no content")
    }
    fn changed_since_capture(&self) -> bool {
        self.changed.load(Ordering::Acquire)
    }
}

fn rendered(work: Work) -> io::Result<CachedFrame> {
    Ok(CachedFrame {
        edge: work.edge,
        frame: Arc::new(Frame {
            id: 0,
            width: 1,
            height: 1,
            frame: work.frame,
            rgba: vec![work.token as u8; 4],
        }),
    })
}

fn cache(root: &tempfile::TempDir) -> Arc<Mutex<Cache>> {
    Arc::new(Mutex::new(
        Cache::open(&root.path().join("cache"), 1024 * 1024, 0).unwrap(),
    ))
}

/// WIN-11: a tile whose file changed after its capture is not left without a
/// thumbnail: its pinned capture is replaced, and the retry after a short delay reads
/// the file again.
#[test]
fn a_file_changed_after_its_capture_is_captured_again_and_its_tile_retried() {
    let root = private_root();
    let first_changed = Arc::new(AtomicBool::new(false));
    let captures = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (flag, counted) = (first_changed.clone(), captures.clone());
    let capture: Capture = Arc::new(move |_| {
        let changed = if counted.fetch_add(1, Ordering::AcqRel) == 0 {
            flag.clone()
        } else {
            Arc::default()
        };
        Ok(Arc::new(Captured { changed }) as Arc<dyn Source>)
    });
    let renders = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = renders.clone();
    let render: Renderer = Arc::new(move |target, _, work, _, cancel| {
        let source = (target.capture)(cancel)?;
        if counted.fetch_add(1, Ordering::AcqRel) == 0 {
            // The file is rewritten while its thumbnail is made.
            first_changed.store(true, Ordering::Release);
        }
        source.validate()?;
        rendered(work)
    });
    let (output, ready) = mpsc::sync_channel(4);
    let mut backend = Backend::start(output, cache(&root), render);
    backend.view(
        1,
        vec![Target {
            capture,
            ..target(1)
        }],
    );
    let started = Instant::now();
    let delivered = ready
        .recv_timeout(crate::scheduler::RETRY_DELAYS[0] + Duration::from_secs(5))
        .expect("the retried tile gets its thumbnail");
    assert!(started.elapsed() >= crate::scheduler::RETRY_DELAYS[0] / 2);
    assert_eq!((delivered.generation, delivered.token), (1, 1));
    assert_eq!(renders.load(Ordering::Acquire), 2);
    assert_eq!(
        captures.load(Ordering::Acquire),
        2,
        "the retry captured the file again"
    );
    assert_eq!(backend.shared.lock().notice.take(), None, "no notice");
    backend.stop();
}

/// A failure that may pass (a busy file, a decoder that timed out) is retried once its
/// short delay has passed, with the same capture.
#[test]
fn a_transient_failure_is_retried_after_a_short_delay() {
    let root = private_root();
    let renders = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = renders.clone();
    let render: Renderer = Arc::new(move |target, _, work, _, cancel| {
        (target.capture)(cancel)?;
        if counted.fetch_add(1, Ordering::AcqRel) == 0 {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Thumbnail decoder timed out",
            ));
        }
        rendered(work)
    });
    let (output, ready) = mpsc::sync_channel(4);
    let mut backend = Backend::start(output, cache(&root), render);
    backend.view(1, vec![target(1)]);
    let delivered = ready
        .recv_timeout(crate::scheduler::RETRY_DELAYS[0] + Duration::from_secs(5))
        .expect("the retried tile gets its thumbnail");
    assert_eq!(delivered.token, 1);
    assert_eq!(renders.load(Ordering::Acquire), 2);
    backend.stop();
}

/// Content no decoder accepts is not decoded again and again while its tile is shown:
/// the tile keeps its file icon. A new view generation tries it once more.
#[test]
fn a_failure_that_cannot_pass_is_not_retried_while_its_tile_is_shown() {
    let root = private_root();
    let renders = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = renders.clone();
    let render: Renderer = Arc::new(move |_, _, _, _, _| {
        counted.fetch_add(1, Ordering::AcqRel);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            crate::decode::Rejected,
        ))
    });
    let (output, ready) = mpsc::sync_channel(4);
    let mut backend = Backend::start(output, cache(&root), render);
    backend.view(1, vec![target(1)]);
    backend.wait_processed(1);
    thread::sleep(crate::scheduler::RETRY_DELAYS[0] + Duration::from_millis(500));
    assert_eq!(
        renders.load(Ordering::Acquire),
        1,
        "never retried in this view"
    );
    backend.view(2, vec![target(1)]);
    backend.wait_processed(2);
    assert_eq!(renders.load(Ordering::Acquire), 2);
    backend.stop();
    assert!(ready.try_recv().is_err());
}

/// WIN-11: a source that fails its revision check while the cache looks up or stores
/// its frame is the job's own failure, never "Could not store or read preview cache".
#[test]
fn a_source_failing_its_check_in_the_cache_is_not_a_cache_failure() {
    struct Tagged(std::sync::atomic::AtomicUsize);
    impl Source for Tagged {
        fn length(&self) -> u64 {
            1
        }
        fn version(&self) -> Vec<u8> {
            vec![1]
        }
        fn validate(&self) -> io::Result<()> {
            // The cache key is made, then the file changes before the lookup.
            if self.0.fetch_add(1, Ordering::AcqRel) == 0 {
                Ok(())
            } else {
                Err(io::Error::other("Captured native content changed."))
            }
        }
        fn read_at(&self, _: u64, _: u32) -> io::Result<Vec<u8>> {
            unreachable!("the lookup fails before any read")
        }
    }
    let root = private_root();
    let cache = Mutex::new(Cache::open(&root.path().join("cache"), 1024 * 1024, 0).unwrap());
    let provider = fixture_provider(root.path().join("decoder-deliberately-absent"));
    let target = Target {
        capture: Arc::new(|_| {
            Ok(Arc::new(Tagged(std::sync::atomic::AtomicUsize::new(0))) as Arc<dyn Source>)
        }),
        ..target(9)
    };
    let work = Work {
        generation: 1,
        token: 9,
        edge: 96,
        frame: 0,
    };
    let Err(error) = render(&target, &provider, work, &cache, &AtomicBool::new(false)) else {
        panic!("a changed source cannot produce a frame");
    };
    assert!(
        !error
            .get_ref()
            .is_some_and(|inner| inner.is::<CacheFailure>()),
        "{error}"
    );
    assert!(crate::cache::is_source_failure(&error), "{error}");
    assert_eq!(error.to_string(), "Captured native content changed.");
}
