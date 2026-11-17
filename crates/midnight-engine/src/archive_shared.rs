//! Request-scoped calls on archive mounts shared by panes, trees, viewers and jobs.
//!
//! The filesystem host ends a cancelled listing between its bounded page exchanges
//! and keeps the worker, and it never ends a shared worker for a cancelled member
//! read either: protocol 3 cancels a read at safe checkpoints, while protocol-1/2
//! workers cannot interrupt a staged read, so the host finishes that exchange and
//! discards the result. Routine request cancellation (a cancelled copy, a revoked tree
//! target) still reaches member reads only through `read_cooperative`, so that the
//! engine never depends on a host's choice for a plain `ArchiveMount::read` whose
//! token is cancelled. Hard tokens remain for mount-owned lifetimes such as a
//! session's own open.
use midnight_core::{ArchiveMount, Cancellation, FsError};

/// Read one bounded member range of a possibly shared mount. A late successful
/// read is discarded once `request` is cancelled; actual provider failures are kept.
pub(crate) fn read(
    mount: &dyn ArchiveMount,
    entry: u64,
    offset: u64,
    length: u32,
    request: &Cancellation,
) -> Result<Vec<u8>, FsError> {
    mount.read_cooperative(entry, offset, length, &|| request.is_cancelled())
}

#[cfg(test)]
pub(crate) mod testing {
    //! Fake archive mount with the filesystem host's cancellation semantics.
    use midnight_core::{ArchiveMount, ArchiveNode, Cancellation, FsError};
    use std::{
        collections::HashMap,
        io,
        path::Path,
        sync::{
            Mutex,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        time::Duration,
    };

    /// A deliberately stricter host than the current filesystem host: a cancelled
    /// listing is discarded and the mount stays usable, but a plain read whose token is
    /// cancelled while the exchange is held terminates the worker, after which every
    /// list/read fails (as earlier hosts did). Engine code must stay correct even
    /// then, so its member reads go through `read_cooperative`, which keeps the
    /// trait's drain-and-discard default here.
    #[derive(Debug, Default)]
    pub(crate) struct SharedMount {
        id: u64,
        rows: Mutex<HashMap<u64, Vec<ArchiveNode>>>,
        contents: Mutex<HashMap<u64, Vec<u8>>>,
        hold: Mutex<Option<Hold>>,
        terminated: AtomicBool,
    }

    #[derive(Debug)]
    struct Hold {
        entered: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
    }

    /// Test side of one held exchange. Dropping it also releases the exchange.
    pub(crate) struct Held {
        entered: mpsc::Receiver<()>,
        release: mpsc::Sender<()>,
    }

    impl Held {
        /// Block until the held call is in flight; the deadline only elapses on failure.
        pub(crate) fn wait_entered(&self) {
            self.entered
                .recv_timeout(Duration::from_secs(10))
                .expect("held archive exchange did not start");
        }

        pub(crate) fn release(self) {
            let _ = self.release.send(());
        }
    }

    fn interrupted() -> FsError {
        FsError::from_io(
            "shared archive",
            Path::new(""),
            io::ErrorKind::Interrupted.into(),
        )
    }

    impl SharedMount {
        pub(crate) fn new(id: u64) -> Self {
            Self {
                id,
                ..Self::default()
            }
        }

        pub(crate) fn insert(&self, directory: u64, rows: Vec<ArchiveNode>) {
            self.rows.lock().unwrap().insert(directory, rows);
        }

        pub(crate) fn insert_content(&self, entry: u64, bytes: &[u8]) {
            self.contents.lock().unwrap().insert(entry, bytes.to_vec());
        }

        /// Hold the next list/read after it has started until the handle is released.
        pub(crate) fn hold_next(&self) -> Held {
            let (entered, started) = mpsc::channel();
            let (release, wait) = mpsc::channel();
            *self.hold.lock().unwrap() = Some(Hold {
                entered,
                release: wait,
            });
            Held {
                entered: started,
                release,
            }
        }

        /// Whether a plain read was cancelled while its exchange was outstanding.
        pub(crate) fn terminated(&self) -> bool {
            self.terminated.load(Ordering::Acquire)
        }

        /// Run one exchange; true when `cancel` was cancelled while it was held.
        fn exchange(&self, cancel: &Cancellation) -> Result<bool, FsError> {
            if self.terminated() || cancel.is_cancelled() {
                return Err(interrupted());
            }
            let hold = self.hold.lock().unwrap().take();
            if let Some(hold) = hold {
                let _ = hold.entered.send(());
                // A dropped test handle disconnects and releases immediately.
                let _ = hold.release.recv_timeout(Duration::from_secs(30));
            }
            Ok(cancel.is_cancelled())
        }
    }

    impl ArchiveMount for SharedMount {
        fn id(&self) -> u64 {
            self.id
        }
        fn list(&self, directory: u64, cancel: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
            if self.exchange(cancel)? {
                return Err(interrupted());
            }
            Ok(self
                .rows
                .lock()
                .unwrap()
                .get(&directory)
                .cloned()
                .unwrap_or_default())
        }
        fn read(
            &self,
            entry: u64,
            offset: u64,
            length: u32,
            cancel: &Cancellation,
        ) -> Result<Vec<u8>, FsError> {
            if self.exchange(cancel)? {
                self.terminated.store(true, Ordering::Release);
                return Err(interrupted());
            }
            let contents = self.contents.lock().unwrap();
            let bytes = contents.get(&entry).map_or(&[][..], Vec::as_slice);
            let start = usize::try_from(offset).unwrap().min(bytes.len());
            let end = start.saturating_add(length as usize).min(bytes.len());
            Ok(bytes[start..end].to_vec())
        }
        fn validate(&self) -> Result<(), FsError> {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::SharedMount;
    use super::*;
    use std::io;

    #[test]
    fn cancelled_read_is_discarded_without_terminating_the_mount() {
        let mount = SharedMount::new(5);
        mount.insert_content(2, b"payload");
        let request = Cancellation::default();
        let held = mount.hold_next();
        let result = std::thread::scope(|scope| {
            let worker = scope.spawn(|| read(&mount, 2, 0, 7, &request));
            held.wait_entered();
            request.cancel();
            held.release();
            worker.join().unwrap()
        });
        assert!(matches!(result, Err(error) if error.kind == io::ErrorKind::Interrupted));
        assert!(
            !mount.terminated(),
            "request cancellation reached the mount"
        );
        assert_eq!(
            read(&mount, 2, 0, 7, &Cancellation::default()).unwrap(),
            b"payload"
        );
    }

    #[test]
    fn the_fake_terminates_on_a_cancelled_plain_read_like_a_protocol_two_worker() {
        let mount = SharedMount::new(5);
        let request = Cancellation::default();
        let held = mount.hold_next();
        let result = std::thread::scope(|scope| {
            let worker = scope.spawn(|| mount.read(2, 0, 7, &request));
            held.wait_entered();
            request.cancel();
            held.release();
            worker.join().unwrap()
        });
        assert!(result.is_err());
        assert!(mount.terminated());
        assert!(mount.list(1, &Cancellation::default()).is_err());
    }
}
