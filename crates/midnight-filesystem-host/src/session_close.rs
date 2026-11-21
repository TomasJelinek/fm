//! A session-specific acknowledgement from the existing independent supervisor.
//! No archive/Copy retirement queue or fresh I/O acquisition participates in this wait.
use super::{
    ACTIVE, MAX_ACTIVE, MAX_SEARCH_ACTIVE, Mount, NESTED_ACTIVE, SEARCH_ACTIVE, Stop, Supervision,
};
use midnight_core::{ArchiveMount, FsError};
use std::{
    io,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
#[cfg(test)]
use std::{
    path::Path,
    time::{Duration, Instant},
};

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
    /// Host references that a failed release retained instead of dropping. Releases
    /// decide under this lock, so they always agree on the last live reference.
    retained: Mutex<usize>,
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
    /// Test-only bounded wait: true once the supervisor confirmed its own teardown.
    #[cfg(test)]
    pub(super) fn closed_within(&self, timeout: Duration) -> bool {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let (state, _) = self
            .changed
            .wait_timeout_while(state, timeout, |state| matches!(state, CloseState::Pending))
            .unwrap_or_else(|e| e.into_inner());
        matches!(*state, CloseState::Closed)
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

/// The allowances of mounts that may be active at once. Each has its own process-wide
/// count, so one use never takes the mounts that another needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Allowance {
    /// Archives opened from native files for panels, tabs and viewers (`MAX_ACTIVE`).
    Native,
    /// Archives inside archives (`nested::MAX_NESTED_ACTIVE`), so that browsing them in
    /// one panel leaves the other panel its archives.
    Nested,
    /// Private sessions of searches inside archives (`MAX_SEARCH_ACTIVE`), so that Find
    /// and the panels never refuse each other's archives.
    Search,
}
impl Allowance {
    /// The process-wide count of active mounts in this allowance, and its limit.
    fn count(self) -> (&'static AtomicUsize, usize) {
        match self {
            Self::Native => (&ACTIVE, MAX_ACTIVE),
            Self::Nested => (&NESTED_ACTIVE, crate::nested::MAX_NESTED_ACTIVE),
            Self::Search => (&SEARCH_ACTIVE, MAX_SEARCH_ACTIVE),
        }
    }
    fn refusal(self, limit: usize) -> io::Error {
        io::Error::other(match self {
            Self::Native => "Two archive mounts are already active; close one first".into(),
            Self::Nested => format!(
                "{limit} archives inside archives are already open; leave one of them first"
            ),
            Self::Search => {
                format!("{limit} archives are already open for searches; stop a search first")
            }
        })
    }
}

pub(super) struct Accounting {
    /// The allowance this mount is counted in.
    allowance: Allowance,
    #[cfg(test)]
    local: Option<Arc<AtomicUsize>>,
}
impl Accounting {
    /// Count one more mount in `allowance`, refused while all of it is in use.
    pub(super) fn reserve(allowance: Allowance) -> io::Result<Self> {
        let (active, limit) = allowance.count();
        if !reserve_in(active, limit) {
            return Err(allowance.refusal(limit));
        }
        Ok(Self {
            allowance,
            #[cfg(test)]
            local: None,
        })
    }
    #[cfg(test)]
    pub(super) fn for_test(local: Arc<AtomicUsize>) -> Self {
        local.fetch_add(1, Ordering::AcqRel);
        Self {
            allowance: Allowance::Native,
            local: Some(local),
        }
    }
}
/// Count one more mount in `active`, unless it already counts `limit`.
fn reserve_in(active: &AtomicUsize, limit: usize) -> bool {
    active
        .try_update(Ordering::AcqRel, Ordering::Acquire, |count| {
            (count < limit).then_some(count + 1)
        })
        .is_ok()
}
impl Drop for Accounting {
    fn drop(&mut self) {
        #[cfg(test)]
        if let Some(local) = &self.local {
            local.fetch_sub(1, Ordering::AcqRel);
            return;
        }
        self.allowance.count().0.fetch_sub(1, Ordering::AcqRel);
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
    /// Return the claims of the staging, wherever it is now, to the count of its spool
    /// volume (`Staging::release_claims`). Its spools stay open.
    fn release_claims(&mut self) {
        if let Some(staging) = self.retired.as_mut() {
            staging.release_claims();
        }
        if let Some(staging) = self
            .shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
        {
            staging.release_claims();
        }
    }
}
/// A teardown that failed keeps the staging armed with every spool open, but its claims
/// are only accounting: they go back first, so later mounts on the same spool volume are
/// never refused for space that this mount will not write ("other archive reads in
/// progress still need"). This never unwinds, as the supervisor must publish its receipt.
fn release_retained_claims(staging: &mut Retained<StagingOwner>) {
    if let Some(staging) = staging.0.as_mut() {
        let released = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            staging.release_claims();
        }));
        if let Err(payload) = released {
            std::mem::forget(payload);
        }
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
        state.wait_for_stop_or_expiry();
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
        Ok(Err(reason)) => {
            release_retained_claims(&mut owner.staging);
            receipt.publish(CloseState::Failed(reason));
        }
        Err(payload) => {
            std::mem::forget(payload);
            release_retained_claims(&mut owner.staging);
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
        let last = {
            let mut retained = receipt.retained.lock().unwrap_or_else(|e| e.into_inner());
            let status = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                mount.get().cleanup_unconfirmed()
            }));
            let failure = match status {
                Ok(false) => None,
                Ok(true) => Some("Archive has an observed unconfirmed cleanup"),
                Err(payload) => {
                    std::mem::forget(payload);
                    Some("Archive cleanup status panicked")
                }
            };
            if let Some(failure) = failure {
                // The incoming reference stays retained and the tainted session stays
                // quarantined, but its supervisor-owned process, staging and permit are
                // independent of the tainted source. Once every remaining reference is
                // retained, no owner can use this session, so let the supervisor end
                // and reclaim them. Every later release observes the latched failure.
                // Engine identities hold only non-upgraded Weak references; a reference
                // dropped without this release is not observed.
                receipt.observe_cleanup(true);
                *retained += 1;
                if Arc::strong_count(mount.get()) == *retained {
                    mount.get().supervision.request_stop(Stop::Closed);
                }
                return Err(fail(failure));
            }
            let incoming = match mount.0.take() {
                Some(mount) => mount,
                None => unreachable!("archive reference already consumed"),
            };
            Arc::into_inner(incoming)
        };
        match last {
            Some(mount) => release_last(mount),
            None => Ok(()),
        }
    }));
    match result {
        Ok(result) => {
            receipt.observe_cleanup(result.is_err());
            result
        }
        Err(payload) => {
            receipt.observe_cleanup(true);
            std::mem::forget(payload);
            Err(fail("Archive consuming teardown panicked"))
        }
    }
}

fn release_last(mount: Mount) -> Result<(), FsError> {
    let mut mount = Retained::new(mount);
    let container = mount.get().container.clone();
    let fail = |message: String| {
        FsError::from_io("release archive", &container, io::Error::other(message))
    };
    mount.get().supervision.request_stop(Stop::Closed);
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
