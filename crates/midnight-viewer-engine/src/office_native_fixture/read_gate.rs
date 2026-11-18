//! A native-test barrier around a real generated capture, never substituted HTML/Data/Denied.
use midnight_plugin_host::{CapturedFile, ContentSource, Error};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Debug)]
pub(super) struct ReadGate {
    pub entered: AtomicBool,
    pub resume: AtomicBool,
    pub delegated: AtomicBool,
    pub delegated_denial: AtomicBool,
    pub timed_out: AtomicBool,
    pub length: AtomicU64,
    pub releases: AtomicUsize,
    closed: Arc<AtomicBool>,
}

impl ReadGate {
    pub(super) fn new(closed: Arc<AtomicBool>) -> Self {
        Self {
            entered: AtomicBool::new(false),
            resume: AtomicBool::new(false),
            delegated: AtomicBool::new(false),
            delegated_denial: AtomicBool::new(false),
            timed_out: AtomicBool::new(false),
            length: AtomicU64::new(0),
            releases: AtomicUsize::new(0),
            closed,
        }
    }
}

#[derive(Debug)]
struct GatedTarget {
    inner: Option<CapturedFile>,
    gate: Arc<ReadGate>,
}

impl GatedTarget {
    fn inner(&self) -> &CapturedFile {
        self.inner
            .as_ref()
            .expect("live gated target retains its real capture")
    }
}

impl ContentSource for GatedTarget {
    fn length(&self) -> u64 {
        self.inner().length()
    }
    fn display_name(&self) -> String {
        self.inner().display_name()
    }
    fn version(&self) -> Vec<u8> {
        self.inner().stable_version()
    }
    fn validate(&self) -> Result<(), Error> {
        self.inner().revalidate()
    }
    fn cleanup_unconfirmed(&self) -> bool {
        self.inner().cleanup_unconfirmed()
    }

    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        self.gate.entered.store(true, Ordering::Release);
        let deadline = Instant::now() + Duration::from_secs(20);
        while !self.gate.resume.load(Ordering::Acquire) && !self.gate.closed.load(Ordering::Acquire)
        {
            if Instant::now() >= deadline {
                self.gate.timed_out.store(true, Ordering::Release);
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        // Always delegate the real checked target read, even if this test gate
        // expired or the window closed. Never fabricate Data or a denial.
        let result = self.inner().read_at(offset, length);
        self.gate
            .delegated_denial
            .store(result.is_err(), Ordering::Release);
        self.gate.delegated.store(true, Ordering::Release);
        result
    }

    fn release_on_worker(self: Arc<Self>) -> Result<(), Error> {
        match Arc::try_unwrap(self) {
            Ok(source) => {
                let mut retained = super::super::Retained::new(source);
                let inner = retained
                    .get_mut()
                    .inner
                    .take()
                    .ok_or_else(|| Error("gated generated target release consumed".into()))?;
                inner.release_on_worker()?;
                retained.get().gate.releases.fetch_add(1, Ordering::AcqRel);
                drop(retained.take());
                Ok(())
            }
            Err(shared) => {
                drop(shared);
                Ok(())
            }
        }
    }
}

pub(super) fn wrap(file: CapturedFile, gate: Arc<ReadGate>) -> Result<CapturedFile, String> {
    gate.length.store(file.length(), Ordering::Release);
    let source = Arc::new(GatedTarget {
        inner: Some(file),
        gate,
    });
    match CapturedFile::from_source(source.clone()) {
        Ok(file) => Ok(file),
        Err(error) => match source.release_on_worker() {
            Ok(()) => Err(error.to_string()),
            Err(cleanup) => Err(format!("{error}; {cleanup}")),
        },
    }
}
