//! Native folder watches over one shared inotify instance per filesystem service.
//!
//! An inotify instance (with its `notify` event thread) is a scarce per-user
//! resource: `fs.inotify.max_user_instances` is often 128 for every process of
//! the user together, so one instance per subscription fails as soon as other
//! programs hold most of them. Every subscription of a `LinuxFileSystem`
//! therefore shares one watcher. A registry maps each subscription to its
//! directories; a directory's native watch is removed when its last
//! subscription ends, and the instance itself stays for later subscriptions.
//! inotify watches inodes, not paths: a registration naming a folder that the
//! shared instance already watches under another path (a bind mount, or a folder
//! renamed since and opened under its new name) gets an instance of its own, so
//! that neither path's watch can end or redirect the other's.
use crate::LinuxFileSystem;
use midnight_core::{FsError, WatchService, WatchSubscription};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    collections::HashMap,
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError, Weak},
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
        let callback = self.0.lock().unwrap_or_else(PoisonError::into_inner).take();
        drop(callback);
    }

    /// `signal` on a watcher's event thread. A consumer callback that panics is never
    /// called again, and the panic ends there: that thread serves every other
    /// subscription, and once it is gone `notify` panics on every later watch change.
    fn deliver(&self, mask: u32) {
        if catch_unwind(AssertUnwindSafe(|| self.signal(mask))).is_err() {
            self.cancel();
        }
    }
}

/// The watcher every subscription of one filesystem service shares.
#[derive(Default)]
pub(crate) struct Hub {
    /// Serializes native watch changes with the directory counts. Held across
    /// calls into the watcher, which wait for its event thread; that thread
    /// never takes it.
    native: Mutex<Native>,
    /// The subscriptions events are dispatched to. Held only briefly: never
    /// across a call into the watcher or a subscriber's callback.
    registry: Arc<Mutex<Registry>>,
}

#[derive(Default)]
struct Native {
    watcher: Option<RecommendedWatcher>,
    /// The directories the shared watcher watches.
    directories: HashMap<PathBuf, Watched>,
    /// The watchers of registrations that alias a directory the shared watcher watches.
    private: HashMap<u64, RecommendedWatcher>,
    next: u64,
    /// Watchers started (one unless starting failed and was retried).
    #[cfg(test)]
    started: usize,
    /// So many of the next starts fail with this error, as the system's
    /// instance limit makes them fail.
    #[cfg(test)]
    fail_starts: Option<(usize, io::ErrorKind)>,
}

/// A natively watched directory.
struct Watched {
    /// How many subscriptions watch it.
    count: usize,
    /// The device and inode its native watch is on; `None` when unknown.
    inode: Option<(u64, u64)>,
}

/// The device and inode `path` names now.
fn inode(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path)
        .ok()
        .map(|metadata| (metadata.dev(), metadata.ino()))
}

#[cfg(test)]
impl Hub {
    /// Watchers started so far.
    fn started(&self) -> usize {
        self.native.lock().unwrap().started
    }

    /// Registrations with a watcher of their own.
    fn private_watchers(&self) -> usize {
        self.native.lock().unwrap().private.len()
    }

    /// Natively watched directories and how many subscriptions watch each.
    fn watched(&self) -> Vec<(PathBuf, usize)> {
        let mut watched: Vec<_> = self
            .native
            .lock()
            .unwrap()
            .directories
            .iter()
            .map(|(path, watched)| (path.clone(), watched.count))
            .collect();
        watched.sort();
        watched
    }

    fn subscriptions(&self) -> usize {
        self.registry.lock().unwrap().subscriptions.len()
    }

    /// The next `count` starts of the shared watcher fail with `kind`.
    fn fail_starts(&self, count: usize, kind: io::ErrorKind) {
        self.native.lock().unwrap().fail_starts = Some((count, kind));
    }
}

#[cfg(test)]
impl Native {
    fn injected_start_failure(&mut self, path: &Path) -> Result<(), FsError> {
        let Some((count, kind)) = self.fail_starts.take() else {
            return Ok(());
        };
        if count > 1 {
            self.fail_starts = Some((count - 1, kind));
        }
        Err(FsError::from_io(
            "start native watcher",
            path,
            io::Error::from(kind),
        ))
    }
}

#[derive(Default)]
struct Registry {
    subscriptions: Vec<Subscription>,
}

struct Subscription {
    id: u64,
    directories: Arc<[PathBuf]>,
    gate: Arc<CallbackGate>,
}

impl Hub {
    /// Watches `directories` (canonical, unique) for `gate` and returns the
    /// registration's id. A failure leaves nothing registered; the next
    /// registration tries again, also to start the shared watcher.
    fn register(
        &self,
        directories: Arc<[PathBuf]>,
        gate: Arc<CallbackGate>,
    ) -> Result<u64, FsError> {
        // The directories the watches will be on, read before the lock.
        let inodes: Vec<_> = directories.iter().map(|path| inode(path)).collect();
        let mut native = self.native.lock().unwrap_or_else(PoisonError::into_inner);
        let native = &mut *native;
        if aliases(&native.directories, &directories, &inodes) {
            let watcher = private_watcher(&directories, &gate)?;
            let id = native.next;
            native.next = native.next.wrapping_add(1);
            native.private.insert(id, watcher);
            return Ok(id);
        }
        let watcher = match native.watcher {
            Some(ref mut watcher) => watcher,
            None => {
                #[cfg(test)]
                native.injected_start_failure(&directories[0])?;
                let watcher = self.start(&directories[0])?;
                #[cfg(test)]
                {
                    native.started += 1;
                }
                native.watcher.insert(watcher)
            }
        };
        // Every registration watches its directories again, also those that
        // other subscriptions watch: inotify watches an inode, and a directory
        // replaced since its watch was added is watched anew. Watching the
        // same directory again adds nothing. The watch of a replaced directory
        // is removed first: `notify` would keep it under the same path, and
        // the removal of the old directory would later end the new watch too.
        for (index, path) in directories.iter().enumerate() {
            if let Some(watched) = native.directories.get_mut(path)
                && watched.inode != inodes[index]
            {
                let _ = watcher.unwatch(path);
                watched.inode = None;
            }
            if let Err(error) = watcher.watch(path, RecursiveMode::NonRecursive) {
                for added in &directories[..index] {
                    if !native.directories.contains_key(added) {
                        let _ = watcher.unwatch(added);
                    }
                }
                return Err(FsError::from_io("watch directory", path, plain(error)));
            }
            if let Some(watched) = native.directories.get_mut(path) {
                watched.inode = inodes[index];
            }
        }
        for (path, inode) in directories.iter().zip(inodes) {
            native
                .directories
                .entry(path.clone())
                .or_insert(Watched { count: 0, inode })
                .count += 1;
        }
        let id = native.next;
        native.next = native.next.wrapping_add(1);
        self.registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .subscriptions
            .push(Subscription {
                id,
                directories,
                gate,
            });
        Ok(id)
    }

    fn start(&self, path: &Path) -> Result<RecommendedWatcher, FsError> {
        notify::recommended_watcher(dispatcher(Arc::downgrade(&self.registry)))
            .map_err(|error| FsError::from_io("start native watcher", path, plain(error)))
    }

    /// Ends registration `id`: its directories no other subscription watches
    /// are no longer watched natively. Waits for the shared event thread.
    fn unregister(&self, id: u64) {
        let mut native = self.native.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(watcher) = native.private.remove(&id) {
            drop(native);
            retire(watcher);
            return;
        }
        let removed = {
            let mut registry = self.registry.lock().unwrap_or_else(PoisonError::into_inner);
            let index = registry
                .subscriptions
                .iter()
                .position(|subscription| subscription.id == id);
            index.map(|index| registry.subscriptions.swap_remove(index))
        };
        let Some(subscription) = removed else {
            return;
        };
        let native = &mut *native;
        for path in subscription.directories.iter() {
            match native.directories.get_mut(path) {
                Some(watched) if watched.count > 1 => watched.count -= 1,
                Some(_) => {
                    native.directories.remove(path);
                    // A directory removed meanwhile is no longer watched. This runs in a
                    // subscription's Drop: a watcher whose event thread is gone, on which
                    // `notify` panics, is given up instead of aborting the process.
                    let unwatched = native.watcher.as_mut().map(|watcher| {
                        catch_unwind(AssertUnwindSafe(|| {
                            let _ = watcher.unwatch(path);
                        }))
                    });
                    if matches!(unwatched, Some(Err(_))) {
                        if let Some(dead) = native.watcher.take() {
                            std::mem::forget(dead);
                        }
                        native.directories.clear();
                        break;
                    }
                }
                None => {}
            }
        }
    }
}

/// The shared watcher's event handler: maps each event to the subscriptions
/// it concerns and signals them. Bounded path comparisons only, without
/// filesystem calls, path cloning or queues; the buffer of due signals is
/// reused. Callbacks run without the registry lock, so a consumer lock a
/// callback takes is never ordered after it.
fn dispatcher(registry: Weak<Mutex<Registry>>) -> impl FnMut(notify::Result<notify::Event>) + Send {
    let mut due: Vec<(Arc<CallbackGate>, u32)> = Vec::new();
    move |event| {
        let Some(registry) = registry.upgrade() else {
            return;
        };
        {
            let registry = registry.lock().unwrap_or_else(PoisonError::into_inner);
            dispatch(&registry.subscriptions, &event, &mut due);
        }
        for (gate, mask) in due.drain(..) {
            gate.deliver(mask);
        }
    }
}

/// The event handler of a registration's own watcher: its events concern it alone.
fn private_dispatcher(
    subscription: Subscription,
) -> impl FnMut(notify::Result<notify::Event>) + Send {
    let subscriptions = [subscription];
    let mut due: Vec<(Arc<CallbackGate>, u32)> = Vec::new();
    move |event| {
        dispatch(&subscriptions, &event, &mut due);
        for (gate, mask) in due.drain(..) {
            gate.deliver(mask);
        }
    }
}

/// Whether a folder of `directories` (with `inodes`) is the same folder as another path
/// that the shared watcher watches, or as another of `directories`: inotify would give
/// both paths one watch.
fn aliases(
    watched: &HashMap<PathBuf, Watched>,
    directories: &[PathBuf],
    inodes: &[Option<(u64, u64)>],
) -> bool {
    directories
        .iter()
        .zip(inodes)
        .enumerate()
        .any(|(index, (path, inode))| {
            inode.is_some_and(|inode| {
                inodes[..index].contains(&Some(inode))
                    || watched
                        .iter()
                        .any(|(other, watched)| other != path && watched.inode == Some(inode))
            })
        })
}

/// A watcher of its own for a registration of `directories` that aliases a folder the
/// shared watcher watches. A failure leaves nothing watched.
fn private_watcher(
    directories: &Arc<[PathBuf]>,
    gate: &Arc<CallbackGate>,
) -> Result<RecommendedWatcher, FsError> {
    let subscription = Subscription {
        id: 0,
        directories: directories.clone(),
        gate: gate.clone(),
    };
    let mut watcher = notify::recommended_watcher(private_dispatcher(subscription))
        .map_err(|error| FsError::from_io("start native watcher", &directories[0], plain(error)))?;
    for path in directories.iter() {
        if let Err(error) = watcher.watch(path, RecursiveMode::NonRecursive) {
            retire(watcher);
            return Err(FsError::from_io("watch directory", path, plain(error)));
        }
    }
    Ok(watcher)
}

/// Ends a watcher. One whose event thread is gone makes `notify` panic on drop; that
/// panic ends here, so that a subscription's Drop never aborts the process.
fn retire(watcher: RecommendedWatcher) {
    let _ = catch_unwind(AssertUnwindSafe(move || drop(watcher)));
}

/// Queues each subscription's change mask for `event` in `due`. An error, an
/// overflow, an event without paths or with more than 64 paths, or a path no
/// subscription watches (the watch of an ended subscription or an unexpected
/// path) signals every directory of every subscription.
fn dispatch(
    subscriptions: &[Subscription],
    event: &notify::Result<notify::Event>,
    due: &mut Vec<(Arc<CallbackGate>, u32)>,
) {
    if !should_refresh(event) {
        return;
    }
    let paths = event_paths(event).filter(|paths| {
        paths.iter().all(|path| {
            subscriptions
                .iter()
                .any(|subscription| bits(&subscription.directories, path) != 0)
        })
    });
    for subscription in subscriptions {
        let mask = match paths {
            Some(paths) => paths
                .iter()
                .fold(0, |mask, path| mask | bits(&subscription.directories, path)),
            None => all(&subscription.directories),
        };
        if mask != 0 {
            due.push((subscription.gate.clone(), mask));
        }
    }
}

/// The paths of an event that names its changes precisely.
fn event_paths(event: &notify::Result<notify::Event>) -> Option<&[PathBuf]> {
    match event {
        Ok(event) if !event.need_rescan() && !event.paths.is_empty() && event.paths.len() <= 64 => {
            Some(&event.paths)
        }
        _ => None,
    }
}

/// Bits of the `targets` that `path` is, or is an immediate entry of.
fn bits(targets: &[PathBuf], path: &Path) -> u32 {
    let mut mask = 0;
    for (index, target) in targets.iter().enumerate() {
        if path == target || path.parent() == Some(target.as_path()) {
            mask |= 1 << index;
        }
    }
    mask
}

fn all(targets: &[PathBuf]) -> u32 {
    u32::MAX >> (32 - targets.len())
}

/// What one subscription's callback receives for `event` when it is the only
/// subscription: the shared watcher's own `dispatch`.
#[cfg(test)]
fn batch_change_mask(targets: &[PathBuf], event: &notify::Result<notify::Event>) -> u32 {
    let subscription = Subscription {
        id: 0,
        directories: Arc::from(targets),
        gate: Arc::new(CallbackGate(Mutex::new(None))),
    };
    let mut due = Vec::new();
    dispatch(std::slice::from_ref(&subscription), event, &mut due);
    due.iter().fold(0, |all, (_, mask)| all | mask)
}

/// A `notify` error as a plain I/O error: what failed, without the debug list
/// of paths `notify` appends (the caller's error names the folder).
fn plain(error: notify::Error) -> io::Error {
    match error.kind {
        notify::ErrorKind::MaxFilesWatch => io::Error::new(
            io::ErrorKind::QuotaExceeded,
            "The system's limit on watched folders is reached (fs.inotify.max_user_watches).",
        ),
        notify::ErrorKind::Io(error) if error.raw_os_error() == Some(libc::EMFILE) => {
            io::Error::new(
                io::ErrorKind::QuotaExceeded,
                "The system's limit on folder watchers (fs.inotify.max_user_instances) or this \
                 process's limit on open files is reached. Midnight Explorer raises its open-file \
                 limit when it starts, to the system's hard limit but at most 65,536.",
            )
        }
        notify::ErrorKind::Io(error) => error,
        notify::ErrorKind::PathNotFound => {
            io::Error::new(io::ErrorKind::NotFound, "The folder does not exist.")
        }
        notify::ErrorKind::Generic(message) => io::Error::other(message),
        notify::ErrorKind::WatchNotFound => io::Error::other("The folder is not watched."),
        notify::ErrorKind::InvalidConfig(_) => {
            io::Error::other("The watch configuration is invalid.")
        }
    }
}

struct NativeWatch {
    hub: Arc<Hub>,
    id: u64,
    gate: Arc<CallbackGate>,
}

impl WatchSubscription for NativeWatch {}

impl Drop for NativeWatch {
    fn drop(&mut self) {
        // Backend ownership waits for an in-flight short callback and releases
        // its captures, then removes only this subscription's directories,
        // which waits for the shared event thread. Never drop under a consumer
        // control lock: that thread may be running another subscription's
        // callback, which can take it.
        self.gate.cancel();
        self.hub.unregister(self.id);
    }
}

impl WatchService for LinuxFileSystem {
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
        let gate = Arc::new(CallbackGate(Mutex::new(Some(changed))));
        match self.watches.register(native_paths, gate.clone()) {
            Ok(id) => Ok(Box::new(NativeWatch {
                hub: self.watches.clone(),
                id,
                gate,
            })),
            Err(error) => {
                gate.cancel();
                Err(error)
            }
        }
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
            .prefix("Midnight Explorer-文档-Žluťoučký-👩🏽‍💻-")
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

#[cfg(test)]
#[path = "watch/batch_tests.rs"]
mod batch_tests;

#[cfg(test)]
#[path = "watch/shared_tests.rs"]
mod shared_tests;
