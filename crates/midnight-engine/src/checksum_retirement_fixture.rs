//! Gated lifetime fixtures shared by checksum adapter/backend regressions.
use midnight_core::*;
use std::{
    io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread::{self, ThreadId},
};

pub(crate) struct Source {
    pub(crate) path: PathBuf,
    pub(crate) resolved: Arc<AtomicUsize>,
    pub(crate) dropped: mpsc::Sender<ThreadId>,
    gate: Option<Arc<DropGate>>,
}
impl Drop for Source {
    fn drop(&mut self) {
        let _ = self.dropped.send(thread::current().id());
        if let Some(gate) = &self.gate {
            gate.wait();
        }
    }
}
impl std::fmt::Debug for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Source")
    }
}
impl CapturedChecksumFiles for Source {
    fn stamp(&self) -> TreeViewStamp {
        TreeViewStamp {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            session: TreeSessionId(1),
            revision: TreeRevision(1),
        }
    }
    fn request(&self) -> TreeRequestId {
        TreeRequestId(1)
    }
    fn resolve(&self, _: &Cancellation) -> Result<Vec<ChecksumTarget>, FsError> {
        self.resolved.fetch_add(1, Ordering::SeqCst);
        Ok(vec![self.path.clone().into()])
    }
}
pub(crate) struct Provider(pub(crate) Option<mpsc::Sender<ThreadId>>);
impl Drop for Provider {
    fn drop(&mut self) {
        if let Some(dropped) = &self.0 {
            let _ = dropped.send(thread::current().id());
        }
    }
}
impl TransferService for Provider {
    fn prepare_transfer_source(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        Err(FsError::from_io(
            "checksum lifetime fixture",
            path,
            io::Error::other("fixture must not read a source"),
        ))
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        Err(FsError::from_io(
            "checksum lifetime fixture",
            path,
            io::Error::other("fixture must not write a destination"),
        ))
    }
}
pub(crate) fn captured(
    job: u64,
    dropped: mpsc::Sender<ThreadId>,
    resolved: Arc<AtomicUsize>,
) -> CapturedChecksumRequest {
    captured_gated(job, dropped, resolved, None)
}
pub(crate) fn captured_gated(
    job: u64,
    dropped: mpsc::Sender<ThreadId>,
    resolved: Arc<AtomicUsize>,
    gate: Option<Arc<DropGate>>,
) -> CapturedChecksumRequest {
    CapturedChecksumRequest {
        job: ChecksumJobId(job),
        generation: ChecksumGeneration(1),
        sources: Arc::new(Source {
            path: std::env::current_dir()
                .unwrap()
                .join("_build/checksum-lifetime-source"),
            resolved,
            dropped,
            gate,
        }),
        algorithms: ChecksumAlgorithms::default(),
        recursive: false,
        limits: ChecksumLimits::default(),
    }
}

pub(crate) struct DropGate {
    pub(crate) entered: tokio::sync::Notify,
    released: std::sync::Mutex<bool>,
    wake: std::sync::Condvar,
}
impl DropGate {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: tokio::sync::Notify::new(),
            released: std::sync::Mutex::new(false),
            wake: std::sync::Condvar::new(),
        })
    }
    pub(crate) fn wait(&self) {
        self.entered.notify_one();
        let mut released = self.released.lock().unwrap();
        while !*released {
            released = self.wake.wait(released).unwrap();
        }
    }
    pub(crate) fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.wake.notify_all();
    }
}
pub(crate) struct ReleaseGate(pub(crate) Arc<DropGate>);
impl Drop for ReleaseGate {
    fn drop(&mut self) {
        self.0.release();
    }
}
