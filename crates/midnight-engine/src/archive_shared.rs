//! Request-scoped calls on archive mounts shared by panes, trees, viewers and jobs.
//!
//! The filesystem host treats the token passed to `ArchiveMount::list`/`read` as the
//! lifetime of that exchange: cancelling it while a reply is outstanding terminates
//! the archive worker and invalidates the mount for every other owner. Routine
//! request cancellation (presence probes while scrolling, tree collapse, quick
//! re-navigation, a cancelled size scan or copy) is therefore never forwarded into a
//! non-cooperative call. It is observed before and after the call; an admitted
//! listing drains within the host's bounds and its late result is discarded. Only an
//! owner-teardown token reaches the provider. Member reads use the cooperative
//! contract, which the host cancels at safe checkpoints without terminating the
//! worker.
use midnight_core::{ArchiveMount, ArchiveNode, Cancellation, FsError};
use std::{io, path::Path};

/// List one directory of a mount that other owners may share.
///
/// `request` is observed only around the provider call: a late successful listing
/// is discarded, while an actual provider failure is returned unchanged so that
/// cancellation cannot erase it. `teardown` is forwarded and must be cancelled only
/// when terminating the whole archive worker is acceptable (owner shutdown); pass
/// `Cancellation::default()` when no such owner exists.
pub(crate) fn list(
    mount: &dyn ArchiveMount,
    directory: u64,
    request: &Cancellation,
    teardown: &Cancellation,
) -> Result<Vec<ArchiveNode>, FsError> {
    check(request)?;
    let rows = mount.list(directory, teardown)?;
    check(request)?;
    Ok(rows)
}

/// Read one bounded member range through the cooperative contract. Request
/// cancellation stops staging at the provider's next safe checkpoint; a late
/// successful read is discarded and actual provider failures are kept.
pub(crate) fn read(
    mount: &dyn ArchiveMount,
    entry: u64,
    offset: u64,
    length: u32,
    request: &Cancellation,
) -> Result<Vec<u8>, FsError> {
    mount.read_cooperative(entry, offset, length, &|| request.is_cancelled())
}

fn check(request: &Cancellation) -> Result<(), FsError> {
    if request.is_cancelled() {
        Err(FsError::from_io(
            "archive request",
            Path::new(""),
            io::ErrorKind::Interrupted.into(),
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod testing {
    //! Fake archive mount with the filesystem host's termination semantics.
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

    /// Like the filesystem host, a token cancelled while its exchange is outstanding
    /// terminates the worker: that call and every later list/read fail. A token that
    /// is already cancelled when a call starts is refused without harming the mount.
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

        /// Whether a request cancellation reached an exchange in flight.
        pub(crate) fn terminated(&self) -> bool {
            self.terminated.load(Ordering::Acquire)
        }

        fn exchange(&self, cancel: &Cancellation) -> Result<(), FsError> {
            let interrupted = || {
                FsError::from_io(
                    "shared archive",
                    Path::new(""),
                    io::ErrorKind::Interrupted.into(),
                )
            };
            if self.terminated() || cancel.is_cancelled() {
                return Err(interrupted());
            }
            let hold = self.hold.lock().unwrap().take();
            if let Some(hold) = hold {
                let _ = hold.entered.send(());
                // A dropped test handle disconnects and releases immediately.
                let _ = hold.release.recv_timeout(Duration::from_secs(30));
            }
            if cancel.is_cancelled() {
                self.terminated.store(true, Ordering::Release);
                return Err(interrupted());
            }
            Ok(())
        }
    }

    impl ArchiveMount for SharedMount {
        fn id(&self) -> u64 {
            self.id
        }
        fn list(&self, directory: u64, cancel: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
            self.exchange(cancel)?;
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
            self.exchange(cancel)?;
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

    #[test]
    fn request_cancellation_drains_a_listing_without_terminating_the_mount() {
        let mount = SharedMount::new(5);
        let request = Cancellation::default();
        let held = mount.hold_next();
        let result = std::thread::scope(|scope| {
            let worker =
                scope.spawn(|| list(&mount, 1, &request, &Cancellation::default()).map(|_| ()));
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
        assert!(mount.list(1, &Cancellation::default()).is_ok());
    }

    #[test]
    fn teardown_still_reaches_the_provider_exchange() {
        let mount = SharedMount::new(5);
        let teardown = Cancellation::default();
        let held = mount.hold_next();
        let result = std::thread::scope(|scope| {
            let worker =
                scope.spawn(|| list(&mount, 1, &Cancellation::default(), &teardown).map(|_| ()));
            held.wait_entered();
            teardown.cancel();
            held.release();
            worker.join().unwrap()
        });
        assert!(result.is_err());
        assert!(mount.terminated(), "owner teardown must stop the worker");
    }

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
}
