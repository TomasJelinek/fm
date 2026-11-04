use crate::LinuxFileSystem;
use fm_core::{FsError, WatchService, WatchSubscription};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    io,
    path::Path,
    sync::{Arc, Mutex},
};

type ChangeCallback = Arc<dyn Fn() + Send + Sync>;

/// Serialize cancellation with the short callback, including an event already in flight.
struct CallbackGate(Mutex<Option<ChangeCallback>>);

impl CallbackGate {
    fn signal(&self) {
        if let Ok(callback) = self.0.lock()
            && let Some(callback) = callback.as_ref()
        {
            callback();
        }
    }

    fn cancel(&self) {
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
        // Engine drops subscriptions on a backend worker. Release the callback before
        // native shutdown, including when inotify is still draining queued events.
        self.gate.cancel();
        drop(self.watcher.take());
    }
}

impl WatchService for LinuxFileSystem {
    fn watch_directory(
        &self,
        path: &Path,
        changed: ChangeCallback,
    ) -> Result<Box<dyn WatchSubscription>, FsError> {
        let metadata = std::fs::metadata(path)
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
        let gate = Arc::new(CallbackGate(Mutex::new(Some(changed))));
        let callback_gate = gate.clone();
        let mut watcher = notify::recommended_watcher(move |event| {
            // Do not copy event paths or read listed files; signal only a bounded hint.
            if should_refresh(&event) {
                callback_gate.signal();
            }
        })
        .map_err(|error| FsError::from_io("start native watcher", path, io::Error::other(error)))?;
        if let Err(error) = watcher.watch(path, RecursiveMode::NonRecursive) {
            gate.cancel();
            return Err(FsError::from_io(
                "watch directory",
                path,
                io::Error::other(error),
            ));
        }
        Ok(Box::new(NativeWatch {
            watcher: Some(watcher),
            gate,
        }))
    }
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
    fn native_watch_does_not_follow_descendant_content_changes() {
        let directory = tempfile::tempdir().unwrap();
        let child = directory.path().join("child").join("grandchild");
        fs::create_dir_all(&child).unwrap();
        let nested_file = child.join("文档.txt");
        fs::write(&nested_file, b"before").unwrap();
        let (sender, receiver) = mpsc::sync_channel(1);
        let subscription = LinuxFileSystem::new(None)
            .watch_directory(
                directory.path(),
                Arc::new(move || {
                    let _ = sender.try_send(());
                }),
            )
            .unwrap();
        quiet(&receiver);
        fs::write(&nested_file, b"after").unwrap();
        assert!(receiver.recv_timeout(Duration::from_millis(150)).is_err());
        fs::write(directory.path().join("immediate.txt"), b"visible").unwrap();
        receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("immediate child notification");
        drop(subscription);
    }

    #[test]
    fn native_unicode_directory_reports_mutations_and_releases_callback_on_drop() {
        let directory = tempfile::Builder::new()
            .prefix("FM-文档-Žluťoučký-👩🏽‍💻-")
            .tempdir()
            .unwrap();
        let provider = LinuxFileSystem::new(None);
        let (sender, receiver) = mpsc::sync_channel(1);
        let callback: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let _ = sender.try_send(());
        });
        let weak = Arc::downgrade(&callback);
        let subscription = provider
            .watch_directory(directory.path(), callback)
            .unwrap();
        let first = directory.path().join("e\u{301}-文档-👩🏽‍💻.txt");
        let renamed = directory.path().join("é-Žluťoučký-👩🏽‍💻.txt");
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
        let _ = fs::read_dir(directory.path()).unwrap().collect::<Vec<_>>();
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
