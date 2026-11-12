//! Observe actual native subscriptions without replacing delivery or debounce policy.
use midnight_core::*;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

pub struct ObservedWatch {
    native: Arc<dyn WatchService>,
    callbacks: Arc<AtomicUsize>,
    batches: Mutex<Vec<Vec<PathBuf>>>,
}

impl WatchService for ObservedWatch {
    fn watch_directory(
        &self,
        path: &Path,
        changed: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Box<dyn WatchSubscription>, FsError> {
        let count = self.callbacks.clone();
        self.native.watch_directory(
            path,
            Arc::new(move || {
                count.fetch_add(1, Ordering::SeqCst);
                changed();
            }),
        )
    }

    fn watch_directories(
        &self,
        paths: &[PathBuf],
        changed: Arc<dyn Fn(u32) + Send + Sync>,
    ) -> Result<Box<dyn WatchSubscription>, FsError> {
        let count = self.callbacks.clone();
        let subscription = self.native.watch_directories(
            paths,
            Arc::new(move |bits| {
                count.fetch_add(1, Ordering::SeqCst);
                changed(bits);
            }),
        )?;
        self.batches.lock().unwrap().push(paths.to_vec());
        Ok(subscription)
    }
}

impl ObservedWatch {
    pub fn new(native: Arc<dyn WatchService>) -> Self {
        Self {
            native,
            callbacks: Arc::new(AtomicUsize::new(0)),
            batches: Mutex::new(Vec::new()),
        }
    }

    pub fn callbacks(&self) -> usize {
        self.callbacks.load(Ordering::SeqCst)
    }

    pub fn observes(&self, path: &Path) -> bool {
        self.batches
            .lock()
            .unwrap()
            .iter()
            .any(|paths| paths.iter().any(|p| p == path))
    }
}
