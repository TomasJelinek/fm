use crate::WindowsFileSystem;
use midnight_core::{FsError, WatchService, WatchSubscription};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

type ChangeCallback = Arc<dyn Fn(u32) + Send + Sync>;

/// Serialize cancellation with the short callback, including an event already in flight.
struct CallbackGate(Mutex<Option<ChangeCallback>>);

impl CallbackGate {
    fn signal(&self, mask: u32) {
        if mask != 0
            && let Ok(callback) = self.0.lock()
            && let Some(callback) = callback.as_ref()
        {
            callback(mask);
        }
    }

    fn cancel(&self) {
        // Poisoning cannot prevent cleanup if a consumer callback panicked.
        let callback = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        drop(callback);
    }
}

struct NativeWatch {
    watcher: Option<RecommendedWatcher>,
    gate: Arc<CallbackGate>,
}

impl WatchSubscription for NativeWatch {}

impl Drop for NativeWatch {
    fn drop(&mut self) {
        // Backend ownership waits for an in-flight short callback, then releases its
        // captures before native shutdown. Never drop under a consumer control lock.
        self.gate.cancel();
        drop(self.watcher.take());
    }
}

impl WatchService for WindowsFileSystem {
    fn watch_directory(
        &self,
        path: &Path,
        changed: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Box<dyn WatchSubscription>, FsError> {
        self.watch_directories(&[path.to_path_buf()], Arc::new(move |_| changed()))
    }

    fn watch_directories(
        &self,
        paths: &[PathBuf],
        changed: ChangeCallback,
    ) -> Result<Box<dyn WatchSubscription>, FsError> {
        let native_paths = registration_paths(paths)?;
        let error_path = &paths[0];
        let gate = Arc::new(CallbackGate(Mutex::new(Some(changed))));
        let callback_gate = gate.clone();
        let callback_paths = native_paths.clone();
        let mut watcher = match notify::recommended_watcher(move |event| {
            // Bounded path comparison only: no filesystem calls, path cloning or queues.
            callback_gate.signal(batch_change_mask(&callback_paths, &event));
        }) {
            Ok(watcher) => watcher,
            Err(error) => {
                gate.cancel();
                return Err(FsError::from_io(
                    "start native watcher",
                    error_path,
                    io::Error::other(error),
                ));
            }
        };
        for path in native_paths.iter() {
            if let Err(error) = watcher.watch(path, RecursiveMode::NonRecursive) {
                gate.cancel();
                // Drop the whole owner, including any paths registered successfully.
                drop(watcher);
                return Err(FsError::from_io(
                    "watch directory",
                    path,
                    io::Error::other(error),
                ));
            }
        }
        Ok(Box::new(NativeWatch {
            watcher: Some(watcher),
            gate,
        }))
    }
}

fn registration_paths(paths: &[PathBuf]) -> Result<Arc<[PathBuf]>, FsError> {
    let error_path = paths.first().map_or(Path::new(""), PathBuf::as_path);
    let invalid = || {
        FsError::from_io(
            "watch directories",
            error_path,
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "expected 1..=32 unique nonempty directories",
            ),
        )
    };
    if paths.is_empty() || paths.len() > 32 {
        return Err(invalid());
    }
    // Reject malformed inputs before performing any native registration.
    for (index, path) in paths.iter().enumerate() {
        if path.as_os_str().is_empty() || paths[..index].contains(path) {
            return Err(invalid());
        }
    }
    let mut native_paths = Vec::with_capacity(paths.len());
    for path in paths {
        // Canonical registration and event identities agree. On Windows std preserves
        // dynamically sized extended Unicode paths without MAX_PATH restrictions.
        let native_path = std::fs::canonicalize(path)
            .map_err(|error| FsError::from_io("resolve watch directory", path, error))?;
        let metadata = std::fs::metadata(&native_path)
            .map_err(|error| FsError::from_io("inspect watch directory", path, error))?;
        if !metadata.is_dir() {
            return Err(FsError::from_io(
                "watch directory",
                path,
                io::Error::new(
                    io::ErrorKind::NotADirectory,
                    "watch target is not a directory",
                ),
            ));
        }
        if native_paths.contains(&native_path) {
            return Err(invalid());
        }
        native_paths.push(native_path);
    }
    Ok(native_paths.into())
}

fn batch_change_mask(targets: &[PathBuf], event: &notify::Result<notify::Event>) -> u32 {
    if !should_refresh(event) {
        return 0;
    }
    let all = u32::MAX >> (32 - targets.len());
    let event = match event {
        Err(_) => return all,
        Ok(event) if event.need_rescan() => return all,
        Ok(event) => event,
    };
    if event.paths.is_empty() || event.paths.len() > 64 {
        return all;
    }
    let mut mask = 0;
    for path in &event.paths {
        let mut matched = false;
        for (index, target) in targets.iter().enumerate() {
            if path == target || path.parent() == Some(target.as_path()) {
                mask |= 1 << index;
                matched = true;
            }
        }
        if !matched {
            return all;
        }
    }
    mask
}

fn should_refresh(event: &notify::Result<notify::Event>) -> bool {
    match event {
        Ok(event) => event.need_rescan() || !event.kind.is_access(),
        Err(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::{
        Event, EventKind,
        event::{AccessKind, CreateKind, Flag, ModifyKind, RemoveKind},
    };
    use std::{
        fs,
        os::windows::ffi::OsStrExt,
        sync::mpsc,
        time::{Duration, Instant},
    };

    #[test]
    fn ignores_access_but_preserves_mutations_overflow_and_errors() {
        assert!(!should_refresh(&Ok(Event::new(EventKind::Access(
            AccessKind::Any
        )))));
        for kind in [
            EventKind::Any,
            EventKind::Create(CreateKind::Any),
            EventKind::Modify(ModifyKind::Any),
            EventKind::Remove(RemoveKind::Any),
            EventKind::Other,
        ] {
            assert!(should_refresh(&Ok(Event::new(kind))));
        }
        assert!(should_refresh(&Ok(Event::new(EventKind::Access(
            AccessKind::Any
        ))
        .set_flag(Flag::Rescan))));
        assert!(should_refresh(&Err(notify::Error::generic("overflow"))));
    }

    fn quiet(receiver: &mpsc::Receiver<()>) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while receiver.recv_timeout(Duration::from_millis(80)).is_ok() {
            assert!(Instant::now() < deadline, "watch did not become quiet");
        }
    }

    #[test]
    fn native_watch_survives_descendant_activity_and_reports_immediate_children() {
        let directory = tempfile::tempdir().unwrap();
        // Windows can report changed metadata for an immediate child directory during
        // descendant activity even when bWatchSubtree is false. Such hints are useful.
        let child = directory.path().join("child").join("grandchild");
        fs::create_dir_all(&child).unwrap();
        let nested_file = child.join("文档.txt");
        fs::write(&nested_file, b"before").unwrap();
        let (sender, receiver) = mpsc::sync_channel(1);
        let subscription = WindowsFileSystem::new(None)
            .watch_directory(
                directory.path(),
                Arc::new(move || {
                    let _ = sender.try_send(());
                }),
            )
            .unwrap();
        quiet(&receiver);
        fs::write(&nested_file, b"after").unwrap();
        quiet(&receiver);
        fs::write(directory.path().join("immediate.txt"), b"visible").unwrap();
        receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("immediate child notification");
        drop(subscription);
    }

    #[test]
    fn native_unicode_directory_reports_mutations_and_releases_callback_on_drop() {
        let directory = tempfile::Builder::new()
            .prefix("Midnight Explorer-文档-Žluťoučký-👩🏽‍💻-")
            .tempdir()
            .unwrap();
        let component = "dlouhá_中文_🧪_".repeat(5);
        let watched = directory
            .path()
            .join(&component)
            .join(&component)
            .join(&component)
            .join(&component);
        assert!(watched.as_os_str().encode_wide().count() > 260);
        fs::create_dir_all(&watched).unwrap();
        let provider = WindowsFileSystem::new(None);
        let (sender, receiver) = mpsc::sync_channel(1);
        let callback: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let _ = sender.try_send(());
        });
        let weak = Arc::downgrade(&callback);
        let subscription = provider.watch_directory(&watched, callback).unwrap();
        let first = watched.join("e\u{301}-文档-👩🏽‍💻.txt");
        let renamed = watched.join("é-Žluťoučký-👩🏽‍💻.txt");
        quiet(&receiver);
        fs::write(&first, b"first").unwrap();
        receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("create notification");
        quiet(&receiver);
        fs::rename(&first, &renamed).unwrap();
        receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("rename notification");
        quiet(&receiver);
        fs::write(&renamed, b"changed contents").unwrap();
        receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("modify notification");
        quiet(&receiver);
        fs::remove_file(&renamed).unwrap();
        receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("delete notification");
        quiet(&receiver);
        let _ = fs::read_dir(&watched).unwrap().collect::<Vec<_>>();
        assert!(
            receiver.recv_timeout(Duration::from_millis(150)).is_err(),
            "listing access caused refresh"
        );
        drop(subscription);
        assert!(
            weak.upgrade().is_none(),
            "subscription retained callback after drop"
        );
        receiver.try_iter().for_each(drop);
        fs::write(first, b"after cancellation").unwrap();
        assert!(receiver.recv_timeout(Duration::from_millis(150)).is_err());
    }
}

#[cfg(test)]
#[path = "watch/batch_tests.rs"]
mod batch_tests;
