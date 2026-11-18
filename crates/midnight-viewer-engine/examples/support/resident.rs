//! Caller-built runtime and external Viewer domain for measurement examples.
use midnight_viewer_engine::{ImageSpareRetirement, Service};
use midnight_work_budget::resident::{Owner, Spawner};
use std::{
    cell::RefCell,
    ops::Deref,
    time::{Duration, Instant},
};

pub(crate) struct ResidentRuntime {
    spare: RefCell<Option<ImageSpareRetirement>>,
    owner: Option<Owner>,
    spawner: Spawner,
    runtime: Option<tokio::runtime::Runtime>,
}

impl ResidentRuntime {
    pub(crate) fn new(runtime: tokio::runtime::Runtime) -> Result<Self, String> {
        let (owner, spawner) = Owner::start(
            "viewer-engine-example",
            midnight_viewer_engine::RESIDENT_WORKERS,
        )
        .map_err(|error| format!("Cannot start Viewer resident workers: {error}"))?;
        Ok(Self {
            spare: RefCell::new(None),
            owner: Some(owner),
            spawner,
            runtime: Some(runtime),
        })
    }

    pub(crate) fn spawner(&self) -> Spawner {
        self.spawner.clone()
    }

    pub(crate) fn track_spare(&self, service: &Service) {
        let mut spare = self.spare.borrow_mut();
        assert!(spare.is_none(), "one Viewer Service per example runtime");
        *spare = Some(service.image_spare_retirement());
    }

    pub(crate) fn shutdown_timeout(mut self, grace: Duration) -> Result<(), String> {
        self.retire(grace)
    }

    fn retire(&mut self, grace: Duration) -> Result<(), String> {
        let Some(runtime) = self.runtime.take() else {
            return Ok(());
        };
        let deadline = Instant::now() + grace;
        // Later-declared windows/services drop first on errors or unwinding.
        // Successful callers close/drop them before consuming this wrapper.
        let spare = self.spare.get_mut().take();
        if let Some(spare) = &spare {
            spare.stop_spare();
        }
        let shutdown = self
            .owner
            .take()
            .map(|owner| owner.shutdown(runtime.handle()));
        let (native, pool) = runtime.block_on(async {
            let at = tokio::time::Instant::from_std(deadline);
            let native = match &spare {
                Some(spare) => Some(tokio::time::timeout_at(at, spare.wait()).await),
                None => None,
            };
            let pool = match shutdown {
                Some(shutdown) => Some(tokio::time::timeout_at(at, shutdown).await),
                None => None,
            };
            (native, pool)
        });
        // Keep the original runtime alive through both checked waits, and the
        // native ticket alive through its timeout/error cleanup as well.
        runtime.shutdown_timeout(deadline.saturating_duration_since(Instant::now()));
        drop(spare);
        let native_ok = native
            .as_ref()
            .is_none_or(|result| matches!(result, Ok(Ok(()))));
        if native_ok && matches!(&pool, Some(Ok(Ok(())))) {
            Ok(())
        } else {
            Err(format!(
                "Viewer example retirement unconfirmed: native={native:?}, pool={pool:?}"
            ))
        }
    }
}

impl Deref for ResidentRuntime {
    type Target = tokio::runtime::Runtime;

    fn deref(&self) -> &Self::Target {
        self.runtime
            .as_ref()
            .expect("original Runtime is available before teardown")
    }
}

impl Drop for ResidentRuntime {
    fn drop(&mut self) {
        if let Err(error) = self.retire(Duration::from_secs(15)) {
            if std::thread::panicking() {
                eprintln!("{error} during unwinding");
            } else {
                panic!("{error}");
            }
        }
    }
}
