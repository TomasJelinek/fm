//! Mount-table notifications for the drive list.
//!
//! The kernel marks an open `/proc/self/mountinfo` with `POLLPRI | POLLERR` whenever
//! the process's mount namespace changes: a USB stick or card mounted by udisks, an
//! unmount, an NFS or CIFS mount, a FUSE mount such as the GVfs bridge itself. Each
//! `poll` that reports the change also acknowledges it, so the table is never read
//! here. One thread blocks in `poll` on that descriptor and an eventfd that stops it;
//! it uses no CPU between changes.
//!
//! Locations inside the GVfs bridge (`/run/user/<id>/gvfs/...`) are folders of one
//! FUSE mount, not mounts, so they send no hint here; the panel asks for the drive
//! list after opening one, and the periodic refresh notices the others.

use midnight_core::{FsError, WatchSubscription};
use rustix::{
    event::{EventfdFlags, PollFd, PollFlags, eventfd, poll},
    fd::OwnedFd,
    fs::{Mode, OFlags, open},
    io::Errno,
};
use std::{io, path::Path, sync::Arc, thread::JoinHandle};

const MOUNTS: &str = "/proc/self/mountinfo";

/// The live watch. Dropping it stops and joins its thread, so the callback is never
/// called afterwards.
pub(crate) struct VolumeWatch {
    stop: OwnedFd,
    thread: Option<JoinHandle<()>>,
}

impl WatchSubscription for VolumeWatch {}

impl Drop for VolumeWatch {
    fn drop(&mut self) {
        // Never blocks: the eventfd is nonblocking and only ever written once.
        let _ = rustix::io::write(&self.stop, &1u64.to_ne_bytes());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Start watching the mount table. Each change calls `changed` once.
pub(crate) fn watch(changed: Arc<dyn Fn() + Send + Sync>) -> Result<VolumeWatch, FsError> {
    let failure =
        |error: Errno| FsError::from_io("watch mounts", Path::new(MOUNTS), io::Error::from(error));
    let mounts = open(MOUNTS, OFlags::RDONLY | OFlags::CLOEXEC, Mode::empty()).map_err(failure)?;
    let stop = eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK).map_err(failure)?;
    let stopped = stop
        .try_clone()
        .map_err(|error| FsError::from_io("watch mounts", Path::new(MOUNTS), error))?;
    let thread = std::thread::Builder::new()
        .name("midnight-mount-watch".into())
        .spawn(move || run(&mounts, &stopped, &*changed))
        .map_err(|error| FsError::from_io("watch mounts", Path::new(MOUNTS), error))?;
    Ok(VolumeWatch {
        stop,
        thread: Some(thread),
    })
}

/// What one `poll` result asks for.
#[derive(Debug, PartialEq, Eq)]
enum Wake {
    Stop,
    Changed,
    Nothing,
}

fn classify(mounts: PollFlags, stop: PollFlags) -> Wake {
    if !stop.is_empty() {
        Wake::Stop
    } else if mounts.intersects(PollFlags::PRI | PollFlags::ERR) {
        Wake::Changed
    } else {
        Wake::Nothing
    }
}

fn run(mounts: &OwnedFd, stop: &OwnedFd, changed: &(dyn Fn() + Send + Sync)) {
    loop {
        let mut descriptors = [
            PollFd::new(mounts, PollFlags::PRI),
            PollFd::new(stop, PollFlags::IN),
        ];
        match poll(&mut descriptors, None) {
            Ok(_) => {}
            Err(Errno::INTR) => continue,
            // The drive list falls back to its periodic refresh.
            Err(_) => return,
        }
        match classify(descriptors[0].revents(), descriptors[1].revents()) {
            Wake::Stop => return,
            Wake::Changed => changed(),
            Wake::Nothing => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        time::{Duration, Instant},
    };

    #[test]
    fn priority_or_error_on_the_mount_table_is_a_change_and_stop_wins() {
        assert_eq!(classify(PollFlags::PRI, PollFlags::empty()), Wake::Changed);
        assert_eq!(
            classify(PollFlags::PRI | PollFlags::ERR, PollFlags::empty()),
            Wake::Changed
        );
        assert_eq!(classify(PollFlags::ERR, PollFlags::empty()), Wake::Changed);
        assert_eq!(
            classify(PollFlags::empty(), PollFlags::empty()),
            Wake::Nothing
        );
        assert_eq!(classify(PollFlags::PRI, PollFlags::IN), Wake::Stop);
    }

    #[test]
    fn an_unchanged_mount_table_sends_nothing_and_dropping_stops_at_once() {
        let hints = Arc::new(AtomicUsize::new(0));
        let counted = hints.clone();
        let watch = watch(Arc::new(move || {
            counted.fetch_add(1, Ordering::SeqCst);
        }))
        .unwrap();
        std::thread::sleep(Duration::from_millis(200));
        let stopping = Instant::now();
        drop(watch);
        assert!(stopping.elapsed() < Duration::from_secs(1));
        assert_eq!(hints.load(Ordering::SeqCst), 0);
    }
}
