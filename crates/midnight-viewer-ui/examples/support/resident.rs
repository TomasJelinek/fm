//! Outer test/probe ownership; caller-supplied Runtime and Viewer stop authority.
use midnight_viewer_engine::{ImageSpareRetirement, Service};
use midnight_work_budget::resident::{Owner, Spawner};
use std::{
    cell::RefCell,
    io,
    ops::Deref,
    time::{Duration, Instant},
};

pub(crate) struct ResidentRuntime {
    spare: RefCell<Option<ImageSpareRetirement>>,
    owner: Option<Owner>,
    spawner: Spawner,
    // Keep the ORIGINAL caller-built runtime alive through native and pool cleanup.
    runtime: Option<tokio::runtime::Runtime>,
}

impl ResidentRuntime {
    pub(crate) fn new(runtime: tokio::runtime::Runtime) -> io::Result<Self> {
        let (owner, spawner) = Owner::start_elastic(
            "viewer-ui-fixture",
            midnight_viewer_engine::RESIDENT_WORKERS,
        )?;
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
        assert!(
            spare.is_none(),
            "one Viewer Service domain per fixture runtime"
        );
        *spare = Some(service.image_spare_retirement());
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
        let Some(runtime) = self.runtime.take() else {
            return;
        };
        let deadline = Instant::now() + Duration::from_secs(15);
        // Callers close sessions/managers first; later-declared locals also drop
        // before this outer guard on every early return. The ticket owns no Service.
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
        // Dispose the same original Runtime with the remainder of the ONE grace,
        // including timeout/error/unwind paths. Keep the native ticket through it.
        runtime.shutdown_timeout(deadline.saturating_duration_since(Instant::now()));
        drop(spare);
        let native_ok = native
            .as_ref()
            .is_none_or(|result| matches!(result, Ok(Ok(()))));
        if !native_ok || !matches!(&pool, Some(Ok(Ok(())))) {
            if std::thread::panicking() {
                eprintln!(
                    "Viewer fixture retirement unconfirmed during unwinding: native={native:?}, pool={pool:?}"
                );
            } else {
                panic!("Viewer fixture retirement unconfirmed: native={native:?}, pool={pool:?}");
            }
        }
    }
}
