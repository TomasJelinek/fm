use super::*;
use notify::{
    Event, EventKind,
    event::{AccessKind, CreateKind, Flag},
};
use std::{fs, sync::mpsc, thread, time::Duration};

fn provider() -> LinuxFileSystem {
    LinuxFileSystem::new(None)
}

#[test]
fn batch_rejects_zero_duplicate_and_33_paths() {
    let directory = tempfile::tempdir().unwrap();
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
fn batch_maps_immediate_children_and_overlapping_targets() {
    let root = PathBuf::from("root");
    let child = root.join("child");
    let sibling = root.join("sibling");
    let targets = [root.clone(), child.clone(), sibling.clone()];
    let event = |path| Ok(Event::new(EventKind::Create(CreateKind::Any)).add_path(path));
    assert_eq!(batch_change_mask(&targets, &event(root.join("file"))), 1);
    assert_eq!(batch_change_mask(&targets, &event(child.clone())), 3);
    assert_eq!(batch_change_mask(&targets, &event(child.join("file"))), 2);
    assert_eq!(batch_change_mask(&targets, &event(sibling.join("file"))), 4);
    let rename = Ok(Event::new(EventKind::Any)
        .add_path(child.join("old"))
        .add_path(sibling.join("new")));
    assert_eq!(batch_change_mask(&targets, &rename), 6);
    assert_eq!(
        batch_change_mask(
            &targets,
            &Ok(Event::new(EventKind::Access(AccessKind::Any)).add_path(root))
        ),
        0
    );
}

#[test]
fn batch_overflow_unknown_and_65_event_paths_signal_all() {
    let targets: Vec<_> = (0..32)
        .map(|index| PathBuf::from(format!("root-{index}")))
        .collect();
    for event in [
        Err(notify::Error::generic("overflow")),
        Ok(Event::new(EventKind::Any)),
        Ok(Event::new(EventKind::Access(AccessKind::Any)).set_flag(Flag::Rescan)),
        Ok(Event::new(EventKind::Any).add_path("unknown".into())),
        Ok(Event::new(EventKind::Any).add_path(PathBuf::new())),
        Ok(Event::new(EventKind::Any)
            .add_path(targets[0].join("known"))
            .add_path("unknown".into())),
    ] {
        assert_eq!(batch_change_mask(&targets, &event), u32::MAX);
    }
    let mut event = Event::new(EventKind::Any);
    for _ in 0..64 {
        event.paths.push(targets[0].join("file"));
    }
    assert_eq!(batch_change_mask(&targets, &Ok(event.clone())), 1);
    event.paths.push(targets[0].join("file"));
    assert_eq!(batch_change_mask(&targets, &Ok(event)), u32::MAX);
    assert_eq!(
        batch_change_mask(
            &targets,
            &Ok(Event::new(EventKind::Any).add_path(targets[31].join("file")))
        ),
        1 << 31
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
        watcher: None,
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

    let directory = tempfile::tempdir().unwrap();
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
    let directory = tempfile::tempdir().unwrap();
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
