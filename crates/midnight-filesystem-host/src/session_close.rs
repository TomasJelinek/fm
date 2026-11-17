//! A session-specific acknowledgement from the existing independent supervisor.
//! No archive/Copy retirement queue or fresh I/O acquisition participates in this wait.
#[cfg(test)]
use super::DEADLINE;
use super::{ACTIVE, Mount, Supervision};
use midnight_core::{ArchiveMount, FsError};
use std::{
    io,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
#[cfg(test)]
use std::{path::Path, sync::atomic::AtomicUsize};

#[derive(Default)]
enum CloseState {
    #[default]
    Pending,
    Closed,
    Failed(String),
}
#[derive(Default)]
pub(super) struct CloseReceipt {
    state: Mutex<CloseState>,
    changed: Condvar,
    // Shared only by this session. Pending/running alone is never cleanup failure.
    unconfirmed: AtomicBool,
}
impl CloseReceipt {
    pub(super) fn new() -> Self {
        Self::default()
    }
    pub(super) fn failed(&self, reason: String) {
        self.publish(CloseState::Failed(reason));
    }
    pub(super) fn observe_cleanup(&self, unconfirmed: bool) {
        if unconfirmed {
            self.unconfirmed.store(true, Ordering::Release);
        }
    }
    pub(super) fn cleanup_unconfirmed(&self) -> bool {
        self.unconfirmed.load(Ordering::Acquire)
    }
    fn publish(&self, state: CloseState) {
        self.observe_cleanup(matches!(&state, CloseState::Failed(_)));
        let mut current = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(*current, CloseState::Pending) {
            *current = state;
        }
        self.changed.notify_all();
    }
    pub(super) fn wait(&self) -> Result<(), String> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            match &*state {
                CloseState::Pending => {
                    state = self.changed.wait(state).unwrap_or_else(|e| e.into_inner());
                }
                CloseState::Closed => return Ok(()),
                CloseState::Failed(reason) => return Err(reason.clone()),
            }
        }
    }
}

/// Kept armed until explicit actual teardown succeeds. Unexpected/error Drop retains
/// the remaining owner rather than invoking another unconfirmed destructor path.
struct Retained<T>(Option<T>);
impl<T> Retained<T> {
    fn new(value: T) -> Self {
        Self(Some(value))
    }
    fn get(&self) -> &T {
        match &self.0 {
            Some(value) => value,
            None => unreachable!("retired native owner"),
        }
    }
    fn get_mut(&mut self) -> &mut T {
        match &mut self.0 {
            Some(value) => value,
            None => unreachable!("retired native owner"),
        }
    }
    fn close(&mut self) {
        drop(self.0.take());
    }
}
impl<T> Drop for Retained<T> {
    fn drop(&mut self) {
        if let Some(value) = self.0.take() {
            std::mem::forget(value);
        }
    }
}

/// The captured input remains armed through validation/startup. Ordinary failures
/// release it explicitly; constructing no Mount must not bypass its content bridge.
pub(super) struct CapturedInput {
    source: Retained<midnight_plugin_host::CapturedFile>,
    unconfirmed: bool,
}
impl CapturedInput {
    pub(super) fn new(source: midnight_plugin_host::CapturedFile) -> Self {
        Self {
            source: Retained::new(source),
            unconfirmed: false,
        }
    }
    pub(super) fn get(&self) -> &midnight_plugin_host::CapturedFile {
        self.source.get()
    }
    pub(super) fn revalidate(&mut self) -> Result<(), midnight_plugin_host::Error> {
        let result = self.get().revalidate();
        self.unconfirmed |= self.get().cleanup_unconfirmed();
        result
    }
    pub(super) fn cleanup_unconfirmed(&self) -> bool {
        self.unconfirmed || self.get().cleanup_unconfirmed()
    }
    pub(super) fn take(&mut self) -> midnight_plugin_host::CapturedFile {
        match self.source.0.take() {
            Some(source) => source,
            None => unreachable!("captured input consumed"),
        }
    }
    pub(super) fn finish_error(&mut self, original: FsError) -> FsError {
        if self.source.0.is_none() {
            return original;
        }
        if self.cleanup_unconfirmed() {
            return FsError::from_io(
                "release archive",
                &original.path,
                io::Error::other(format!(
                    "Captured archive source has an observed unconfirmed cleanup; original failure: {original}"
                )),
            );
        }
        let source = self.take();
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| source.release_on_worker()));
        let reason = match result {
            Ok(Ok(())) => return original,
            Ok(Err(error)) => format!("Captured archive source teardown unconfirmed: {error}"),
            Err(payload) => {
                std::mem::forget(payload);
                "Captured archive source teardown panicked".into()
            }
        };
        FsError::from_io(
            "release archive",
            &original.path,
            io::Error::other(format!("{reason}; original failure: {original}")),
        )
    }
}

pub(super) struct Accounting {
    #[cfg(test)]
    local: Option<Arc<AtomicUsize>>,
}
impl Accounting {
    pub(super) fn reserve() -> io::Result<Self> {
        ACTIVE
            .try_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < 2).then_some(active + 1)
            })
            .map_err(|_| {
                io::Error::other("Two archive mounts are already active; close one first")
            })?;
        Ok(Self {
            #[cfg(test)]
            local: None,
        })
    }
    #[cfg(test)]
    fn for_test(local: Arc<AtomicUsize>) -> Self {
        local.fetch_add(1, Ordering::AcqRel);
        Self { local: Some(local) }
    }
}
impl Drop for Accounting {
    fn drop(&mut self) {
        #[cfg(test)]
        if let Some(local) = &self.local {
            local.fetch_sub(1, Ordering::AcqRel);
            return;
        }
        ACTIVE.fetch_sub(1, Ordering::AcqRel);
    }
}

// Private, finite native composition: the production implementation is the existing
// worker-platform Process. Tests substitute its kill/wait boundary without a child app.
pub(super) trait SupervisorProcess: Send {
    fn kill(&mut self) -> io::Result<()>;
    fn wait_terminated(&mut self) -> io::Result<()>;
}
impl SupervisorProcess for midnight_worker_platform::Process {
    fn kill(&mut self) -> io::Result<()> {
        midnight_worker_platform::Process::kill(self)
    }
    fn wait_terminated(&mut self) -> io::Result<()> {
        midnight_worker_platform::Process::wait_terminated(self)
    }
}
struct StagingOwner {
    shared: Arc<Mutex<Option<crate::staging::Staging>>>,
    retired: Option<crate::staging::Staging>,
}
impl StagingOwner {
    fn take(&mut self) {
        self.retired = self.shared.lock().unwrap_or_else(|e| e.into_inner()).take();
    }
    fn close(&mut self) {
        drop(self.retired.take());
    }
}
pub(super) struct Supervisor<P: SupervisorProcess = midnight_worker_platform::Process> {
    process: Retained<P>,
    staging: Retained<StagingOwner>,
    accounting: Retained<Accounting>,
}
impl<P: SupervisorProcess> Supervisor<P> {
    pub(super) fn new(
        process: P,
        staging: Arc<Mutex<Option<crate::staging::Staging>>>,
        accounting: Accounting,
    ) -> Self {
        Self {
            process: Retained::new(process),
            staging: Retained::new(StagingOwner {
                shared: staging,
                retired: None,
            }),
            accounting: Retained::new(accounting),
        }
    }
}

pub(super) fn supervise<P: SupervisorProcess>(
    owner: Supervisor<P>,
    state: Arc<Supervision>,
    receipt: Arc<CloseReceipt>,
) {
    supervise_with(owner, state, receipt, || {});
}
fn supervise_with<P: SupervisorProcess>(
    mut owner: Supervisor<P>,
    state: Arc<Supervision>,
    receipt: Arc<CloseReceipt>,
    before_staging_drop: impl FnOnce(),
) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        while !state.stop.load(Ordering::Acquire) && !state.expired_at(Instant::now()) {
            std::thread::sleep(Duration::from_millis(10));
        }
        state.stop.store(true, Ordering::Release);
        // Kill may race an already terminated child. Only the positive native wait
        // below, never the kill result, certifies termination (including nonzero exits).
        let _ = owner.process.get_mut().kill();
        owner
            .process
            .get_mut()
            .wait_terminated()
            .map_err(|error| format!("Archive native wait unconfirmed: {error}"))?;
        owner.staging.get_mut().take();
        // The staging mutex has been released. The owner is still armed through the
        // actual content-file destruction and every following native destructor.
        before_staging_drop();
        owner.staging.get_mut().close();
        owner.staging.close();
        owner.process.close();
        owner.accounting.close();
        Ok::<(), String>(())
    }));
    match result {
        Ok(Ok(())) => receipt.publish(CloseState::Closed),
        Ok(Err(reason)) => receipt.publish(CloseState::Failed(reason)),
        Err(payload) => {
            std::mem::forget(payload);
            receipt.publish(CloseState::Failed(
                "Archive supervisor teardown panicked".into(),
            ));
        }
    }
    // Any unresolved process/staging/accounting remains in armed Retained storage.
}

pub(super) fn release(mount: Arc<Mount>) -> Result<(), FsError> {
    // Cached status forwards one opaque data-source edge. Arm the incoming reference
    // before that callback so even its unwind cannot ordinary-Drop the session owner.
    let mut mount = Retained::new(mount);
    let container = mount.get().container.clone();
    let receipt = mount.get().close.clone();
    let fail = |message: &str| {
        FsError::from_io(
            "release archive",
            &container,
            io::Error::other(message.to_owned()),
        )
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if mount.get().cleanup_unconfirmed() {
            return Err(fail("Archive has an observed unconfirmed cleanup"));
        }
        let incoming = match mount.0.take() {
            Some(mount) => mount,
            None => unreachable!("archive reference already consumed"),
        };
        let Some(mount) = Arc::into_inner(incoming) else {
            return Ok(());
        };
        release_last(mount)
    }));
    match result {
        Ok(result) => {
            receipt.observe_cleanup(result.is_err());
            result
        }
        Err(payload) => {
            receipt.observe_cleanup(true);
            std::mem::forget(payload);
            Err(fail(
                "Archive cleanup status or consuming teardown panicked",
            ))
        }
    }
}

fn release_last(mount: Mount) -> Result<(), FsError> {
    let mut mount = Retained::new(mount);
    let container = mount.get().container.clone();
    let fail = |message: String| {
        FsError::from_io("release archive", &container, io::Error::other(message))
    };
    mount.get().supervision.stop.store(true, Ordering::Release);
    let receipt = mount.get().close.clone();
    receipt.wait().map_err(fail)?;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if let Some(source) = mount.get_mut().source.take() {
            source
                .release_on_worker()
                .map_err(|error| fail(format!("Archive source teardown unconfirmed: {error}")))?;
        }
        // The supervisor receipt covers its owners, not these remaining captured
        // content/channel/native fields. Actual destruction still runs on this worker.
        mount.close();
        Ok(())
    }));
    match result {
        Ok(result) => {
            receipt.observe_cleanup(result.is_err());
            result
        }
        Err(payload) => {
            receipt.observe_cleanup(true);
            std::mem::forget(payload);
            Err(fail("Archive remaining native teardown panicked".into()))
        }
    }
}

#[cfg(test)]
#[path = "session_close_tests.rs"]
mod tests;
