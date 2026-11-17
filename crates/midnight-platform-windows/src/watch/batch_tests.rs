use super::*;
use crate::test_fixtures::build_temp;
use std::{
    fs,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

fn provider() -> WindowsFileSystem {
    WindowsFileSystem::new(None)
}

/// Waits (bounded) until `expected` arrives, ignoring other masks.
fn expect_mask(receiver: &mpsc::Receiver<u32>, expected: u32) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(left) {
            Ok(mask) if mask == expected => return,
            Ok(_) => {}
            Err(_) => panic!("mask {expected:#b} was never signalled"),
        }
    }
}

#[test]
fn batch_rejects_zero_duplicate_and_33_paths() {
    let directory = build_temp("watch-batch-");
    let root = directory.path().to_path_buf();
    let callback: Arc<dyn Fn(u32) + Send + Sync> = Arc::new(|_| {});
    for paths in [
        vec![],
        vec![root.clone(), root.clone()],
        vec![PathBuf::new()],
    ] {
        assert!(
            matches!(provider().watch_directories(&paths, callback.clone()), Err(error) if error.kind == io::ErrorKind::InvalidInput)
        );
    }
    let mut paths: Vec<_> = (0..32)
        .map(|index| root.join(format!("directory-{index}")))
        .collect();
    for path in &paths {
        fs::create_dir(path).unwrap();
    }
    let subscription = provider()
        .watch_directories(&paths, callback.clone())
        .unwrap();
    drop(subscription);
    paths.push(root);
    assert!(
        matches!(provider().watch_directories(&paths, callback), Err(error) if error.kind == io::ErrorKind::InvalidInput)
    );
    let aliases = vec![paths[0].clone(), paths[0].join(".")];
    assert!(
        matches!(provider().watch_directories(&aliases, Arc::new(|_| {})), Err(error) if error.kind == io::ErrorKind::InvalidInput)
    );
}

#[test]
fn batch_cancel_waits_for_short_callback_and_teardown_releases_owner() {
    let (entered_sender, entered_receiver) = mpsc::sync_channel(1);
    let (release_sender, release_receiver) = mpsc::sync_channel(1);
    let release_receiver = Mutex::new(release_receiver);
    let callback: Arc<dyn Fn(u32) + Send + Sync> = Arc::new(move |mask| {
        entered_sender.send(mask).unwrap();
        release_receiver
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
    });
    let weak = Arc::downgrade(&callback);
    let gate = Arc::new(CallbackGate(Mutex::new(Some(callback))));
    let signal_gate = gate.clone();
    let signal = thread::spawn(move || signal_gate.signal(5));
    assert_eq!(
        entered_receiver
            .recv_timeout(Duration::from_secs(5))
            .unwrap(),
        5
    );
    let subscription = NativeWatch {
        registration: None,
        gate: gate.clone(),
    };
    let (done_sender, done_receiver) = mpsc::sync_channel(1);
    let cancel = thread::spawn(move || {
        drop(subscription);
        done_sender.send(()).unwrap();
    });
    assert!(
        done_receiver
            .recv_timeout(Duration::from_millis(50))
            .is_err()
    );
    release_sender.send(()).unwrap();
    signal.join().unwrap();
    cancel.join().unwrap();
    assert!(weak.upgrade().is_none());
    gate.signal(1);
    assert!(entered_receiver.try_recv().is_err());

    let directory = build_temp("watch-teardown-");
    let (sender, receiver) = mpsc::sync_channel(8);
    let callback: Arc<dyn Fn(u32) + Send + Sync> = Arc::new(move |mask| {
        let _ = sender.try_send(mask);
    });
    let weak = Arc::downgrade(&callback);
    let subscription = provider()
        .watch_directories(&[directory.path().into()], callback)
        .unwrap();
    drop(subscription);
    assert!(weak.upgrade().is_none());
    receiver.try_iter().for_each(drop);
    fs::write(directory.path().join("after-drop"), b"after").unwrap();
    assert!(receiver.recv_timeout(Duration::from_millis(150)).is_err());
}

#[test]
fn batch_native_child_maps_to_registered_slot_and_failure_releases_callback() {
    let directory = build_temp("watch-slots-");
    let first = directory.path().join("first-文档");
    let second = directory.path().join("second-Žluťoučký");
    fs::create_dir(&first).unwrap();
    fs::create_dir(&second).unwrap();
    let (sender, receiver) = mpsc::sync_channel(16);
    let callback: Arc<dyn Fn(u32) + Send + Sync> = Arc::new(move |mask| {
        let _ = sender.try_send(mask);
    });
    let subscription = provider()
        .watch_directories(&[first.clone(), second.clone()], callback)
        .unwrap();
    fs::write(second.join("e\u{301}-👩🏽‍💻.txt"), b"visible").unwrap();
    assert_eq!(receiver.recv_timeout(Duration::from_secs(5)).unwrap(), 2);
    drop(subscription);
    let callback: Arc<dyn Fn(u32) + Send + Sync> = Arc::new(|_| {});
    let weak = Arc::downgrade(&callback);
    assert!(
        provider()
            .watch_directories(&[first, second.join("missing")], callback)
            .is_err()
    );
    assert!(weak.upgrade().is_none());
}

#[test]
fn batch_overlapping_target_child_change_also_signals_that_target() {
    let directory = build_temp("watch-overlap-");
    let root = directory.path().to_path_buf();
    let child = root.join("child");
    fs::create_dir(&child).unwrap();
    let (sender, receiver) = mpsc::sync_channel(16);
    let subscription = provider()
        .watch_directories(
            &[root.clone(), child.clone()],
            Arc::new(move |mask| {
                let _ = sender.try_send(mask);
            }),
        )
        .unwrap();
    // A change to the registered child directory's own entry (reported by the parent
    // watch) marks both the parent and that child target.
    fs::create_dir(root.join("unrelated")).unwrap();
    expect_mask(&receiver, 1);
    use std::os::windows::fs::OpenOptionsExt;
    let entry = fs::OpenOptions::new()
        .access_mode(windows_sys::Win32::Storage::FileSystem::FILE_WRITE_ATTRIBUTES)
        .custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS)
        .open(&child)
        .unwrap();
    entry
        .set_modified(std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000))
        .unwrap();
    drop(entry);
    expect_mask(&receiver, 3);
    drop(subscription);
}

/// Before the owned service, a kernel buffer overflow completed as an empty batch that
/// produced no event, so a burst of changes was lost until the 15 s poll.
#[test]
fn batch_overflow_signals_every_target_and_keeps_watching() {
    let directory = build_temp("watch-overflow-");
    let busy = directory.path().join("busy");
    let other = directory.path().join("other");
    fs::create_dir(&busy).unwrap();
    fs::create_dir(&other).unwrap();
    let hold = Arc::new(AtomicBool::new(true));
    let (entered, entered_receiver) = mpsc::sync_channel(1);
    let (release, release_receiver) = mpsc::sync_channel::<()>(1);
    let release_receiver = Mutex::new(release_receiver);
    let (sender, receiver) = mpsc::sync_channel(64);
    let held = hold.clone();
    let subscription = provider()
        .watch_directories(
            &[busy.clone(), other.clone()],
            Arc::new(move |mask| {
                if held.swap(false, Ordering::SeqCst) {
                    // Hold the service thread so no read is pending while changes arrive.
                    let _ = entered.send(mask);
                    let _ = release_receiver
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(10));
                } else {
                    let _ = sender.try_send(mask);
                }
            }),
        )
        .unwrap();
    fs::write(busy.join("first"), b"first").unwrap();
    assert_eq!(
        entered_receiver
            .recv_timeout(Duration::from_secs(5))
            .unwrap(),
        1
    );
    // Far more than the 64 KiB notification buffer of change records.
    let stem = "x".repeat(120);
    for index in 0..1500 {
        fs::File::create(busy.join(format!("overflow-{index:04}-{stem}"))).unwrap();
    }
    release.send(()).unwrap();
    expect_mask(&receiver, 3);
    // The overflowed directory is re-armed and still reports ordinary changes.
    receiver.try_iter().for_each(drop);
    fs::write(other.join("after"), b"after").unwrap();
    expect_mask(&receiver, 2);
    drop(subscription);
}

/// Before the owned service, an error completion (here: the watched directory was
/// removed) silently unwatched the directory without any event.
#[test]
fn batch_removed_directory_signals_every_target_and_others_keep_watching() {
    let directory = build_temp("watch-removed-");
    let removed = directory.path().join("removed");
    let kept = directory.path().join("kept");
    fs::create_dir(&removed).unwrap();
    fs::create_dir(&kept).unwrap();
    let (sender, receiver) = mpsc::sync_channel(64);
    let subscription = provider()
        .watch_directories(
            &[removed.clone(), kept.clone()],
            Arc::new(move |mask| {
                let _ = sender.try_send(mask);
            }),
        )
        .unwrap();
    fs::remove_dir(&removed).unwrap();
    expect_mask(&receiver, 3);
    receiver.try_iter().for_each(drop);
    fs::write(kept.join("still watched"), b"kept").unwrap();
    expect_mask(&receiver, 2);
    drop(subscription);
}
