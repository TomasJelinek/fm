//! Headless ownership/scheduling regressions; native provider qualification stays separate.
use super::*;
use midnight_core::{DirectoryIndex, Entry, EntryId, EntryKind, GeneratedTextSource};
use midnight_work_budget::resident::Owner;
use std::{
    io,
    path::PathBuf,
    sync::{Condvar, mpsc},
    thread::ThreadId,
    time::Duration,
};

const WAIT: Duration = Duration::from_secs(5);

struct Gate(Arc<(Mutex<bool>, Condvar)>);

impl Drop for Gate {
    fn drop(&mut self) {
        let (state, changed) = &*self.0;
        *state.lock().unwrap_or_else(|error| error.into_inner()) = true;
        changed.notify_all();
    }
}

fn hold_all(
    spawner: &midnight_work_budget::resident::Spawner,
    handle: &tokio::runtime::Handle,
) -> (Gate, Vec<ThreadId>) {
    let gate = Gate(Arc::new((Mutex::new(false), Condvar::new())));
    let (entered, entries) = mpsc::sync_channel(RESIDENT_WORKERS);
    for _ in 0..RESIDENT_WORKERS {
        let state = gate.0.clone();
        let entered = entered.clone();
        assert!(
            spawner
                .spawn(
                    handle,
                    Box::new(move || {
                        let _ = entered.try_send(std::thread::current().id());
                        let (state, changed) = &*state;
                        let state = state.lock().unwrap_or_else(|error| error.into_inner());
                        drop(
                            changed
                                .wait_while(state, |released| !*released)
                                .unwrap_or_else(|error| error.into_inner()),
                        );
                    }),
                )
                .is_ok()
        );
    }
    let threads = (0..RESIDENT_WORKERS)
        .map(|_| entries.recv_timeout(WAIT).unwrap())
        .collect();
    (gate, threads)
}

#[derive(Debug)]
struct TrackedSource {
    dropped: mpsc::Sender<(ThreadId, usize)>,
    windows: Arc<AtomicUsize>,
}

impl GeneratedTextSource for TrackedSource {
    fn length(&self) -> u64 {
        0
    }
    fn read_at(&self, _: u64, _: &mut [u8]) -> io::Result<usize> {
        panic!("an unstarted viewer must not read its generated source")
    }
}

impl Drop for TrackedSource {
    fn drop(&mut self) {
        let _ = self.dropped.send((
            std::thread::current().id(),
            self.windows.load(Ordering::Acquire),
        ));
    }
}

fn launch(service: &Service, dropped: mpsc::Sender<(ThreadId, usize)>) -> ViewerLaunch {
    let path = PathBuf::from("resident-owned.txt");
    let entries: Arc<[Entry]> = vec![Entry::new(EntryId(1), path.clone(), EntryKind::File)].into();
    ViewerLaunch {
        find: None,
        associations: Default::default(),
        tree: None,
        archive: None,
        generated: Some(Arc::new(TrackedSource {
            dropped,
            windows: service.windows.clone(),
        })),
        focused: path,
        view: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
        preferences: Arc::default(),
        basic: true,
        disabled_plugins: Arc::from([]),
    }
}

fn appearance() -> ViewerAppearance {
    ViewerAppearance {
        family: "monospace".into(),
        font_size: 14.0,
        foreground: [240; 4],
        background: [20, 20, 20, 255],
        selection: [70; 4],
        selection_text: [0; 4],
        accent: [180; 4],
    }
}

#[test]
fn skipped_queued_window_drops_source_on_resident_before_returning_window_credit() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (owner, spawner) = Owner::start("viewer-skip-test", RESIDENT_WORKERS).unwrap();
    let (gate, threads) = hold_all(&spawner, runtime.handle());
    let service = Service::new(runtime.handle().clone(), spawner);
    let spare = service.image_spare_retirement();
    let (dropped, drops) = mpsc::channel();
    let session = service
        .open(
            launch(&service, dropped),
            appearance(),
            ViewerGeometry {
                width: 320,
                height: 200,
                scale: 1.0,
            },
        )
        .unwrap();
    assert_eq!(service.windows.load(Ordering::Acquire), 1);
    session.close();
    owner.seal();
    drop(gate);
    let (thread, windows_at_source_drop) = drops.recv_timeout(WAIT).unwrap();
    assert!(threads.contains(&thread));
    assert_eq!(windows_at_source_drop, 1);
    runtime.block_on(async {
        tokio::time::timeout(WAIT, owner.shutdown(runtime.handle()))
            .await
            .unwrap()
            .unwrap();
        assert!(
            tokio::time::timeout(WAIT, spare.wait())
                .await
                .unwrap()
                .is_ok()
        );
    });
    assert_eq!(service.windows.load(Ordering::Acquire), 0);
}

/// A window job that the resident pool drops without running (sealed while it waited
/// behind busy workers) returns its admitted slot and fails the opening. Opening after
/// the transient runtime is gone is outside the Service contract (see below).
#[test]
fn window_slot_and_opening_fail_cleanly_when_the_resident_pool_drops_the_worker() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .max_blocking_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (owner, spawner) = Owner::start("viewer-dropped-worker-test", RESIDENT_WORKERS).unwrap();
    let (gate, _threads) = hold_all(&spawner, runtime.handle());
    let service = Service::with_native_capture(
        runtime.handle().clone(),
        spawner,
        Arc::new(|_, _| panic!("a dropped worker captures nothing")),
    );
    let (dropped, _drops) = mpsc::channel();
    let session = service
        .open(
            launch(&service, dropped),
            appearance(),
            ViewerGeometry {
                width: 200,
                height: 100,
                scale: 1.0,
            },
        )
        .unwrap();
    assert_eq!(service.windows.load(Ordering::Acquire), 1);
    owner.seal();
    drop(gate);
    let deadline = std::time::Instant::now() + WAIT;
    while service.windows.load(Ordering::Acquire) != 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "the admitted window slot is returned"
        );
        std::thread::yield_now();
    }
    assert_eq!(
        session.mailbox.take_open_result(),
        Some(Err("Viewer backend is unavailable.".into()))
    );
    runtime.block_on(async {
        tokio::time::timeout(WAIT, owner.shutdown(runtime.handle()))
            .await
            .unwrap()
            .unwrap();
    });
}

#[test]
fn refused_window_retires_its_owned_envelope_off_caller_before_returning_credit() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (owner, spawner) = Owner::start("viewer-refusal-test", RESIDENT_WORKERS).unwrap();
    owner.seal();
    let service = Service::new(runtime.handle().clone(), spawner);
    let (dropped, drops) = mpsc::channel();
    let result = service.open(
        launch(&service, dropped),
        appearance(),
        ViewerGeometry {
            width: 320,
            height: 200,
            scale: 1.0,
        },
    );
    assert!(result.is_err());
    let (thread, windows_at_source_drop) = drops.recv_timeout(WAIT).unwrap();
    assert_ne!(thread, std::thread::current().id());
    assert_eq!(windows_at_source_drop, 1);
    let deadline = std::time::Instant::now() + WAIT;
    while service.windows.load(Ordering::Acquire) != 0 {
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    runtime.block_on(owner.shutdown(runtime.handle())).unwrap();
}

#[test]
fn idle_image_spare_does_not_occupy_the_only_transient_blocking_slot() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let (owner, spawner) = Owner::start("viewer-spare-test", RESIDENT_WORKERS).unwrap();
    let service = Service::new(runtime.handle().clone(), spawner);
    let spare = service.image_spare_retirement();
    let (sent, received) = mpsc::sync_channel(1);
    runtime.spawn_blocking(move || {
        let _ = sent.try_send(());
    });
    received.recv_timeout(WAIT).unwrap();
    spare.stop_spare();
    runtime.block_on(async {
        tokio::time::timeout(WAIT, spare.wait())
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(WAIT, owner.shutdown(runtime.handle()))
            .await
            .unwrap()
            .unwrap();
    });
}

#[test]
fn runtime_exit_does_not_move_queued_window_destruction_to_the_runtime_caller() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let (owner, spawner) = Owner::start("viewer-runtime-exit-test", RESIDENT_WORKERS).unwrap();
    let (gate, threads) = hold_all(&spawner, runtime.handle());
    let service = Service::new(runtime.handle().clone(), spawner);
    let (dropped, drops) = mpsc::channel();
    let session = service
        .open(
            launch(&service, dropped),
            appearance(),
            ViewerGeometry {
                width: 320,
                height: 200,
                scale: 1.0,
            },
        )
        .unwrap();
    session.close();
    owner.seal();
    drop(runtime);
    drop(gate);
    let (thread, windows_at_source_drop) = drops.recv_timeout(WAIT).unwrap();
    assert!(threads.contains(&thread));
    assert_ne!(thread, std::thread::current().id());
    assert_eq!(windows_at_source_drop, 1);
    let deadline = std::time::Instant::now() + WAIT;
    while service.windows.load(Ordering::Acquire) != 0 {
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    // This observes queued capture/credit disposal after original runtime exit.
    // Common executor tests separately qualify worker-exit/join completion.
    // Submitting NEW UI work after transient teardown is outside Service's contract.
    drop(owner);
}

#[test]
fn queued_window_accepts_latest_palette_through_full_wakes_before_opening_or_close() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (owner, spawner) = Owner::start("viewer-palette-queue-test", RESIDENT_WORKERS).unwrap();
    let (gate, threads) = hold_all(&spawner, runtime.handle());
    let service = Service::new(runtime.handle().clone(), spawner);
    let (dropped, drops) = mpsc::channel();
    let a = appearance();
    let session = service
        .open(
            launch(&service, dropped),
            a.clone(),
            ViewerGeometry {
                width: 320,
                height: 200,
                scale: 1.0,
            },
        )
        .unwrap();
    assert_eq!(session.palette_revision(), 1);
    for _ in 0..input_queue::INPUTS {
        assert!(session.send(ViewerInput::RetainedRaster(false)));
    }
    assert!(!session.send(ViewerInput::RetainedRaster(false)));
    assert!(session.wakes.wake());
    let preferences = session.preferences();
    let mut b = a.clone();
    b.background = [250; 4];
    b.family = "UI only".into();
    b.font_size = 72.0;
    assert_eq!(session.restore_palette(&b).unwrap(), 2);
    let accepted = session.restore_palette(&a).unwrap();
    assert_eq!(accepted, 3);
    assert_eq!(session.palette_revision(), accepted);
    assert!(Arc::ptr_eq(&session.preferences(), &preferences));
    assert!(session.mailbox.take_open_result().is_none());
    session.close();
    assert_eq!(session.palette_revision(), accepted);
    assert!(session.restore_palette(&b).is_err());
    assert_eq!(session.palette_revision(), accepted);
    owner.seal();
    drop(gate);
    let (thread, windows_at_source_drop) = drops.recv_timeout(WAIT).unwrap();
    assert!(threads.contains(&thread));
    assert_eq!(windows_at_source_drop, 1);
    runtime.block_on(owner.shutdown(runtime.handle())).unwrap();
    assert_eq!(service.windows.load(Ordering::Acquire), 0);
}

#[test]
fn disconnected_window_keeps_accepted_palette_revision_when_later_setter_rejects() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (owner, spawner) =
        Owner::start("viewer-palette-disconnect-test", RESIDENT_WORKERS).unwrap();
    let (gate, threads) = hold_all(&spawner, runtime.handle());
    let service = Service::new(runtime.handle().clone(), spawner);
    let (dropped, drops) = mpsc::channel();
    let a = appearance();
    let session = service
        .open(
            launch(&service, dropped),
            a.clone(),
            ViewerGeometry {
                width: 320,
                height: 200,
                scale: 1.0,
            },
        )
        .unwrap();
    assert_eq!(session.palette_revision(), 1);
    let mut b = a.clone();
    b.accent = [10, 20, 30, 255];
    let accepted = session.restore_palette(&b).unwrap();
    assert_eq!(accepted, 2);

    // All resident workers are held, so sealing deterministically skips the
    // queued real window. Its receiver/lifetime drop without Session::close.
    owner.seal();
    drop(gate);
    runtime.block_on(async {
        tokio::time::timeout(WAIT, owner.shutdown(runtime.handle()))
            .await
            .unwrap()
            .unwrap();
    });
    let (thread, windows_at_source_drop) = drops.recv_timeout(WAIT).unwrap();
    assert!(threads.contains(&thread));
    assert_eq!(windows_at_source_drop, 1);
    assert_eq!(service.windows.load(Ordering::Acquire), 0);
    assert!(!session.closed.load(Ordering::Acquire));
    assert!(!session.send(ViewerInput::RetainedRaster(false)));
    assert_eq!(session.palette_revision(), accepted);
    assert!(session.restore_palette(&a).is_err());
    assert_eq!(session.palette_revision(), accepted);
    session.close();
    assert_eq!(session.palette_revision(), accepted);
}
