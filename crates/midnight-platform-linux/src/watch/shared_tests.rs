//! Every subscription of a filesystem service shares one inotify instance.
use super::*;
use notify::{Event, EventKind, event::CreateKind};
use std::{
    fs,
    os::unix::fs::MetadataExt,
    sync::mpsc,
    time::{Duration, Instant},
};

/// The inotify descriptors of this process that natively watch `directory`,
/// read from `/proc/self/fdinfo` (one `ino:` line per watch).
fn inotify_fds_watching(directory: &Path) -> Vec<String> {
    let inode = format!("ino:{:x} ", fs::metadata(directory).unwrap().ino());
    let mut fds = Vec::new();
    for entry in fs::read_dir("/proc/self/fd").unwrap().flatten() {
        let fd = entry.file_name().to_string_lossy().into_owned();
        let is_inotify = fs::read_link(entry.path())
            .is_ok_and(|target| target.to_string_lossy() == "anon_inode:inotify");
        let watches = fs::read_to_string(format!("/proc/self/fdinfo/{fd}")).unwrap_or_default();
        if is_inotify && watches.contains(&inode) {
            fds.push(fd);
        }
    }
    fds
}

fn masks() -> (ChangeCallback, mpsc::Receiver<u32>) {
    let (sender, receiver) = mpsc::sync_channel(64);
    (
        Arc::new(move |mask| {
            let _ = sender.try_send(mask);
        }),
        receiver,
    )
}

/// The masks received until 300 ms pass without one.
fn settled(receiver: &mpsc::Receiver<u32>) -> u32 {
    let mut mask = 0;
    let deadline = Instant::now() + Duration::from_secs(5);
    while let Ok(next) = receiver.recv_timeout(Duration::from_millis(300)) {
        mask |= next;
        assert!(Instant::now() < deadline, "notifications never settled");
    }
    mask
}

#[test]
fn subscriptions_share_one_instance_and_count_the_directories_they_share() {
    let root = tempfile::tempdir().unwrap();
    let [a, b, c] = ["a-文档", "b", "c"].map(|name| {
        let path = fs::canonicalize(root.path()).unwrap().join(name);
        fs::create_dir(&path).unwrap();
        path
    });
    let provider = LinuxFileSystem::new(None);
    let (first_callback, first) = masks();
    let first_owner = Arc::downgrade(&first_callback);
    let first_watch = provider
        .watch_directories(&[a.clone(), b.clone()], first_callback)
        .unwrap();
    let (second_callback, second) = masks();
    let second_watch = provider
        .watch_directories(&[b.clone(), c.clone()], second_callback)
        .unwrap();
    let (third_callback, third) = masks();
    let third_watch = provider
        .watch_directory(&a, Arc::new(move || third_callback(1)))
        .unwrap();
    assert_eq!(
        provider.watches.started(),
        1,
        "one watcher for every subscription"
    );
    assert_eq!(
        provider.watches.watched(),
        [(a.clone(), 2), (b.clone(), 2), (c.clone(), 1)]
    );
    // One inotify descriptor watches every directory.
    let fds = [&a, &b, &c].map(|directory| inotify_fds_watching(directory));
    assert_eq!(fds[0].len(), 1, "{fds:?}");
    assert!(fds.iter().all(|fd| *fd == fds[0]), "{fds:?}");

    // Each subscription hears its own directories, with its own bits.
    for receiver in [&first, &second, &third] {
        settled(receiver);
    }
    fs::write(b.join("new"), b"b").unwrap();
    assert_eq!(settled(&first), 0b10);
    assert_eq!(settled(&second), 0b01);
    assert_eq!(settled(&third), 0);

    // Ending a subscription releases only its own directories and callback.
    drop(first_watch);
    assert!(first_owner.upgrade().is_none());
    assert_eq!(
        provider.watches.watched(),
        [(a.clone(), 1), (b.clone(), 1), (c.clone(), 1)]
    );
    fs::write(b.join("again"), b"b").unwrap();
    assert_eq!(
        settled(&second),
        0b01,
        "the shared directory is still watched"
    );
    fs::write(a.join("new"), b"a").unwrap();
    assert_eq!(settled(&third), 1);
    assert_eq!(settled(&first), 0);

    // The last subscription of a directory removes its native watch.
    drop(second_watch);
    assert_eq!(provider.watches.watched(), [(a.clone(), 1)]);
    assert!(inotify_fds_watching(&b).is_empty());
    assert!(inotify_fds_watching(&c).is_empty());
    drop(third_watch);
    assert!(provider.watches.watched().is_empty());
    assert_eq!(provider.watches.subscriptions(), 0);
    assert!(inotify_fds_watching(&a).is_empty());

    // The instance stays for later subscriptions.
    let (callback, later) = masks();
    let watch = provider
        .watch_directories(std::slice::from_ref(&c), callback)
        .unwrap();
    assert_eq!(provider.watches.started(), 1);
    settled(&later);
    fs::write(c.join("later"), b"c").unwrap();
    assert_eq!(settled(&later), 1);
    drop(watch);
}

#[test]
fn a_directory_replaced_while_watched_is_watched_anew_by_the_next_subscription() {
    let root = tempfile::tempdir().unwrap();
    let folder = fs::canonicalize(root.path()).unwrap().join("folder");
    fs::create_dir(&folder).unwrap();
    let provider = LinuxFileSystem::new(None);
    let (old_callback, old) = masks();
    let old_watch = provider
        .watch_directories(std::slice::from_ref(&folder), old_callback)
        .unwrap();
    // The folder is replaced while the old subscription still holds it.
    let replaced = root.path().join("replaced");
    fs::rename(&folder, &replaced).unwrap();
    fs::create_dir(&folder).unwrap();
    settled(&old);
    let (new_callback, new) = masks();
    let new_watch = provider
        .watch_directories(std::slice::from_ref(&folder), new_callback)
        .unwrap();
    assert_eq!(provider.watches.watched(), [(folder.clone(), 2)]);
    assert_eq!(inotify_fds_watching(&folder).len(), 1);
    assert!(
        inotify_fds_watching(&replaced).is_empty(),
        "the replaced folder's watch was removed"
    );
    // Removing the old folder does not end the new folder's watch.
    fs::remove_dir(&replaced).unwrap();
    settled(&new);
    drop(old_watch);
    fs::write(folder.join("inside the new folder"), b"new").unwrap();
    assert_eq!(settled(&new), 1);
    drop(new_watch);
    assert!(provider.watches.watched().is_empty());
    assert!(inotify_fds_watching(&folder).is_empty());
}

#[test]
fn a_watcher_that_cannot_start_is_started_by_a_later_registration() {
    let root = tempfile::tempdir().unwrap();
    let provider = LinuxFileSystem::new(None);
    provider
        .watches
        .fail_starts(2, io::ErrorKind::QuotaExceeded);
    let (callback, receiver) = masks();
    let owner = Arc::downgrade(&callback);
    for _ in 0..2 {
        let Err(error) = provider.watch_directories(&[root.path().into()], callback.clone()) else {
            panic!("the instance limit refuses the watcher");
        };
        assert_eq!(error.operation, "start native watcher");
        assert_eq!(error.kind, io::ErrorKind::QuotaExceeded);
    }
    assert_eq!(provider.watches.started(), 0);
    assert_eq!(provider.watches.subscriptions(), 0);
    let watch = provider
        .watch_directories(&[root.path().into()], callback)
        .unwrap();
    assert_eq!(provider.watches.started(), 1);
    settled(&receiver);
    fs::write(root.path().join("after the retry"), b"live").unwrap();
    assert_eq!(settled(&receiver), 1);
    drop(watch);
    assert!(
        owner.upgrade().is_none(),
        "no failed attempt kept the callback"
    );
}

#[test]
fn a_failed_directory_leaves_the_shared_watches_as_they_were() {
    let root = tempfile::tempdir().unwrap();
    let kept = fs::canonicalize(root.path()).unwrap().join("kept");
    fs::create_dir(&kept).unwrap();
    let provider = LinuxFileSystem::new(None);
    let (callback, receiver) = masks();
    let watch = provider
        .watch_directories(std::slice::from_ref(&kept), callback)
        .unwrap();
    let fresh = root.path().join("fresh");
    fs::create_dir(&fresh).unwrap();
    let removed = root.path().join("removed");
    fs::create_dir(&removed).unwrap();
    // `removed` vanishes after its path is resolved, so its native watch fails.
    let paths: Arc<[PathBuf]> = Arc::from(vec![
        fs::canonicalize(&kept).unwrap(),
        fs::canonicalize(&fresh).unwrap(),
        fs::canonicalize(&removed).unwrap(),
    ]);
    fs::remove_dir(&removed).unwrap();
    let gate = Arc::new(CallbackGate(Mutex::new(None)));
    assert!(provider.watches.register(paths, gate).is_err());
    assert_eq!(provider.watches.watched(), [(kept.clone(), 1)]);
    assert!(
        inotify_fds_watching(&fresh).is_empty(),
        "the rollback unwatched it"
    );
    settled(&receiver);
    fs::write(kept.join("still watched"), b"kept").unwrap();
    assert_eq!(settled(&receiver), 1);
    drop(watch);
}

#[test]
fn shared_dispatch_signals_only_the_subscriptions_an_event_concerns() {
    let subscription = |id: u64, directories: &[&str]| {
        let (callback, receiver) = masks();
        (
            Subscription {
                id,
                directories: directories.iter().map(PathBuf::from).collect(),
                gate: Arc::new(CallbackGate(Mutex::new(Some(callback)))),
            },
            receiver,
        )
    };
    let (first, first_masks) = subscription(1, &["/a", "/b"]);
    let (second, second_masks) = subscription(2, &["/b", "/c"]);
    let subscriptions = [first, second];
    let fired = |event: notify::Result<Event>| {
        let mut due = Vec::new();
        dispatch(&subscriptions, &event, &mut due);
        for (gate, mask) in due.drain(..) {
            gate.signal(mask);
        }
        (
            first_masks.try_iter().fold(0, |all, mask| all | mask),
            second_masks.try_iter().fold(0, |all, mask| all | mask),
        )
    };
    let created =
        |path: &str| Ok(Event::new(EventKind::Create(CreateKind::Any)).add_path(path.into()));
    assert_eq!(fired(created("/a/file")), (0b01, 0));
    assert_eq!(fired(created("/b/file")), (0b10, 0b01));
    assert_eq!(fired(created("/c")), (0, 0b10));
    // Errors, overflow and paths no subscription watches signal everything.
    assert_eq!(fired(Err(notify::Error::generic("overflow"))), (0b11, 0b11));
    assert_eq!(fired(created("/elsewhere/file")), (0b11, 0b11));
}

#[test]
fn watch_limit_errors_read_plainly_without_debug_paths() {
    let watches =
        plain(notify::Error::new(notify::ErrorKind::MaxFilesWatch).add_path("/tmp".into()));
    assert_eq!(watches.kind(), io::ErrorKind::QuotaExceeded);
    let text = watches.to_string();
    assert!(text.contains("max_user_watches"), "{text}");
    assert!(!text.contains("about") && !text.contains("/tmp"), "{text}");
    let instances = plain(notify::Error::io(io::Error::from_raw_os_error(
        libc::EMFILE,
    )));
    assert!(
        instances.to_string().contains("max_user_instances"),
        "{instances}"
    );
}

/// inotify watches inodes. A folder renamed while watched, then watched under its new name
/// (the engine registers the new target before it drops the old one), would share the old
/// name's watch, and dropping the old subscription would end it for both. The new name gets
/// a watcher of its own and keeps hearing its changes.
#[test]
fn a_renamed_folder_watched_under_its_new_name_keeps_hearing_changes_after_the_old_name_ends() {
    let root = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(root.path()).unwrap();
    let old = base.join("old");
    let new = base.join("new");
    fs::create_dir(&old).unwrap();
    let provider = LinuxFileSystem::new(None);
    let (old_callback, _old_masks) = masks();
    let old_watch = provider
        .watch_directories(std::slice::from_ref(&old), old_callback)
        .unwrap();
    fs::rename(&old, &new).unwrap();
    let (new_callback, new_masks) = masks();
    let new_watch = provider
        .watch_directories(std::slice::from_ref(&new), new_callback)
        .unwrap();
    assert_eq!(
        provider.watches.private_watchers(),
        1,
        "the alias watches alone"
    );
    assert_eq!(provider.watches.started(), 1);
    drop(old_watch);
    settled(&new_masks);
    fs::write(new.join("file"), b"after the old name ended").unwrap();
    assert_eq!(
        settled(&new_masks),
        1,
        "the new name still hears its changes"
    );
    drop(new_watch);
    assert_eq!(provider.watches.private_watchers(), 0);
}

/// A consumer callback that panics is not called again, and the event thread that every
/// other subscription shares survives it: later changes are still heard, and dropping
/// subscriptions and registering new ones still work.
#[test]
fn a_panicking_callback_never_stops_the_shared_watcher() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let root = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(root.path()).unwrap();
    let [a, b] = ["a", "b"].map(|name| {
        let path = base.join(name);
        fs::create_dir(&path).unwrap();
        path
    });
    let provider = LinuxFileSystem::new(None);
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let panicking = provider
        .watch_directories(
            std::slice::from_ref(&a),
            Arc::new(move |_| {
                counted.fetch_add(1, Ordering::SeqCst);
                panic!("consumer callback fixture");
            }),
        )
        .unwrap();
    let (callback, other_masks) = masks();
    let other = provider
        .watch_directories(std::slice::from_ref(&b), callback)
        .unwrap();
    fs::write(a.join("first"), b"").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while calls.load(Ordering::SeqCst) == 0 {
        assert!(Instant::now() < deadline, "the callback never ran");
        std::thread::sleep(Duration::from_millis(5));
    }
    fs::write(a.join("second"), b"").unwrap();
    settled(&other_masks);
    fs::write(b.join("file"), b"").unwrap();
    assert_eq!(settled(&other_masks), 1, "the shared event thread survived");
    assert_eq!(calls.load(Ordering::SeqCst), 1, "never called again");
    drop(panicking);
    drop(other);
    let (callback, again_masks) = masks();
    let again = provider
        .watch_directories(std::slice::from_ref(&a), callback)
        .unwrap();
    fs::write(a.join("third"), b"").unwrap();
    assert_eq!(settled(&again_masks), 1);
    assert_eq!(provider.watches.started(), 1, "the same watcher serves on");
    drop(again);
}
