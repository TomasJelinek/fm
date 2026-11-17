//! Copy cleanup and native destruction retain the same armed authority envelope.
use super::*;
use std::{
    collections::VecDeque,
    ops::{Deref, DerefMut},
};

#[path = "copy_seal.rs"]
pub(super) mod seal;
use crate::copy_nested_retirement::{CheckedRetirement, Context as RetirementContext};
pub(super) use seal::Armed;
pub(crate) use seal::Quarantined;

/// Provider-free delivery may keep the original admission bound after teardown.
pub(crate) struct AdmissionTail {
    _capacity: OwnedSemaphorePermit,
}

pub(super) struct Resources {
    receipt: Option<crate::copy_fence::Receipt>,
    finished: Option<crate::copy_finished::Owner>,
    predecessors: crate::copy_finished::Predecessors,
    published: bool,
    job_cancel: Option<Cancellation>,
    pub mutation: Option<OwnedSemaphorePermit>,
    pub real_io: Option<Arc<midnight_work_budget::Permit>>,
    io_gate: Option<Arc<midnight_work_budget::Gate>>,
    namespace: Option<crate::copy_namespace::Claim>,
    status: Option<Status>,
    namespace_errors: Vec<FsError>,
    capacity: Arc<AdmissionTail>,
    native: VecDeque<Box<dyn CheckedRetirement>>,
    providers: VecDeque<Box<dyn CheckedRetirement>>,
    unknown: VecDeque<Box<dyn Send>>,
    retirement: RetirementContext,
}
impl Resources {
    pub(super) fn admission(
        receipt: crate::copy_fence::Receipt,
        finished: crate::copy_finished::Owner,
        capacity: OwnedSemaphorePermit,
        predecessors: crate::copy_finished::Predecessors,
    ) -> Self {
        Self {
            receipt: Some(receipt),
            finished: Some(finished),
            predecessors,
            published: true,
            job_cancel: None,
            mutation: None,
            real_io: None,
            io_gate: None,
            namespace: None,
            status: None,
            namespace_errors: Vec::new(),
            capacity: Arc::new(AdmissionTail {
                _capacity: capacity,
            }),
            native: VecDeque::new(),
            providers: VecDeque::new(),
            unknown: VecDeque::new(),
            retirement: RetirementContext::default(),
        }
    }
    pub(super) fn unpublished(capacity: OwnedSemaphorePermit, cancel: Cancellation) -> Self {
        let mut resources = Self::admission(
            crate::copy_fence::Receipt::unpublished(),
            crate::copy_finished::Owner::unpublished(),
            capacity,
            crate::copy_finished::Predecessors::with_capacity(),
        );
        resources.published = false;
        resources.job_cancel = Some(cancel);
        resources
    }
    pub(super) fn pending_cancel(&self) -> Option<Cancellation> {
        self.job_cancel.clone()
    }
    pub(super) fn finished_observer(&self) -> crate::copy_finished::FinishedObserver {
        self.finished
            .as_ref()
            .expect("Copy has a strong finished owner")
            .observer()
    }
    pub(super) fn publish_admission(
        &mut self,
        sequence: u64,
        finished: &mut crate::copy_finished::Registry,
        fence: &mut crate::copy_fence::FenceRegistration<'_>,
    ) {
        finished.publish_prepared(
            sequence,
            self.finished
                .as_mut()
                .expect("unpublished Copy has a strong owner"),
            &mut self.predecessors,
        );
        fence.publish(
            self.receipt
                .as_ref()
                .expect("unpublished Copy has a receipt"),
        );
        self.published = true;
    }
    pub(super) fn take_predecessors(&mut self) -> crate::copy_finished::Predecessors {
        std::mem::take(&mut self.predecessors)
    }
    pub(super) fn admission_sequence(&self) -> u64 {
        self.finished
            .as_ref()
            .expect("one strong Copy finished owner")
            .sequence()
    }
    pub(super) fn attach_status(&mut self, status: Status) {
        self.status = Some(status);
    }
    pub(super) fn attach_io(
        &mut self,
        gate: Arc<midnight_work_budget::Gate>,
        permit: Arc<midnight_work_budget::Permit>,
    ) -> Result<(), FsError> {
        if !permit.belongs_to(&gate)
            || !permit.is_shared()
            || self
                .namespace
                .as_ref()
                .is_some_and(|claim| !Arc::ptr_eq(&gate, &claim.configured_io()))
            || self.real_io.is_some()
        {
            return Err(FsError::from_io(
                "attach transfer I/O",
                Path::new(""),
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Copy requires one shared permit from its configured I/O gate.",
                ),
            ));
        }
        self.io_gate = Some(gate);
        self.real_io = Some(permit);
        Ok(())
    }
    pub(super) fn install_namespace(
        &mut self,
        claim: crate::copy_namespace::Claim,
        native: crate::copy_namespace::NativeOwners,
    ) -> Result<(), FsError> {
        if self.namespace.is_some()
            || self
                .io_gate
                .as_ref()
                .is_some_and(|gate| !Arc::ptr_eq(gate, &claim.configured_io()))
            || self.real_io.as_ref().is_some_and(|permit| {
                !permit.belongs_to(&claim.configured_io()) || !permit.is_shared()
            })
        {
            claim.quarantine();
            std::mem::forget(native);
            std::mem::forget(claim);
            return Err(FsError::from_io(
                "install transfer namespace",
                Path::new(""),
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "A Copy scope already owns its namespace claim.",
                ),
            ));
        }
        self.namespace = Some(claim);
        self.retain_native(Box::new(native));
        Ok(())
    }
    pub(super) fn namespace(&self) -> Option<&crate::copy_namespace::Claim> {
        self.namespace.as_ref()
    }
    pub(super) fn delivery_tail(&self) -> Arc<AdmissionTail> {
        self.capacity.clone()
    }
    pub(super) fn retain_native(&mut self, value: Box<dyn CheckedRetirement>) {
        self.native.push_back(value);
    }
    #[cfg(test)]
    pub(super) fn retain_provider(&mut self, value: Box<dyn Send>) {
        self.unknown.push_back(value);
        self.retirement.quarantine();
    }
    pub(super) fn retain_checked_provider(&mut self, value: Box<dyn CheckedRetirement>) {
        self.providers.push_back(value);
    }
    pub(super) fn receipt_observer(&self) -> crate::copy_fence::PendingReceipt {
        self.receipt
            .as_ref()
            .expect("Copy receipt remains owned")
            .observer()
    }
}

struct AuthoritySeal {
    resources: Option<Box<Resources>>,
}
/// A call-local clone of the same real permit, destroyed before seal commit.
/// Neither installed TLS context retains this clone or crosses an await.
struct CallContext {
    gate: Arc<midnight_work_budget::Gate>,
    permit: Arc<midnight_work_budget::Permit>,
    changes:
        Option<Arc<dyn midnight_platform_common::transfer_namespace::TransferNamespaceChanges>>,
}
impl CallContext {
    fn invoke<R>(&self, operation: impl FnOnce() -> R) -> Result<R, Quarantined> {
        midnight_platform_common::transfer_io::with_transfer_io_permit_from(
            &self.gate,
            &self.permit,
            || {
                if let Some(changes) = &self.changes {
                    midnight_platform_common::transfer_namespace::with_transfer_namespace_changes(
                        changes.clone(),
                        operation,
                    )
                } else {
                    operation()
                }
            },
        )
        .map_err(|_| Quarantined)
    }
}
impl AuthoritySeal {
    fn new(resources: Resources) -> Self {
        Self {
            resources: Some(Box::new(resources)),
        }
    }
    fn resources_mut(&mut self) -> &mut Resources {
        self.resources
            .as_mut()
            .expect("Copy authority remains sealed")
    }
    fn call_context(&self) -> Result<Option<CallContext>, Quarantined> {
        let resources = self
            .resources
            .as_ref()
            .expect("Copy authority remains sealed");
        let gate = resources.io_gate.clone().or_else(|| {
            resources
                .namespace
                .as_ref()
                .map(crate::copy_namespace::Claim::configured_io)
        });
        let Some(gate) = gate else {
            return Ok(None);
        };
        let permit = resources.real_io.as_ref().ok_or(Quarantined)?;
        if !permit.belongs_to(&gate) || !permit.is_shared() {
            return Err(Quarantined);
        }
        Ok(Some(CallContext {
            gate,
            permit: permit.clone(),
            changes: resources
                .namespace
                .as_ref()
                .map(crate::copy_namespace::Claim::changes),
        }))
    }
    fn native_call<R>(&self, operation: impl FnOnce() -> R) -> Result<R, Quarantined> {
        if self.is_quarantined() {
            std::mem::forget(operation);
            return Err(Quarantined);
        }
        let context = match self.call_context() {
            Ok(context) => context,
            Err(error) => {
                std::mem::forget(operation);
                return Err(error);
            }
        };
        let retirement = self
            .resources
            .as_ref()
            .expect("Copy authority remains sealed")
            .retirement
            .clone();
        let result = seal::checked(|| {
            retirement.with(|| match &context {
                Some(context) => context.invoke(operation),
                None => Ok(operation()),
            })
        })
        .and_then(|result| result);
        drop(context);
        if result.is_err() {
            self.quarantine();
        }
        result
    }
    fn is_quarantined(&self) -> bool {
        self.resources
            .as_ref()
            .is_some_and(|resources| resources.retirement.is_quarantined())
            || self
                .resources
                .as_ref()
                .and_then(|resources| resources.namespace.as_ref())
                .is_some_and(crate::copy_namespace::Claim::is_quarantined)
    }
    fn quarantine(&self) {
        if let Some(resources) = &self.resources {
            // SUCCESS and FINISHED have distinct meanings. An uncertain native
            // outcome can never satisfy SUCCESS, while FINISHED remains pending
            // until actual teardown is confirmed and its authority can release.
            if let Some(receipt) = &resources.receipt {
                receipt.observer().fail();
            }
            resources.retirement.quarantine();
            if let Some(status) = &resources.status {
                let errors = resources.retirement.take_errors();
                let mut progress = status.lock().unwrap_or_else(|error| error.into_inner());
                for error in errors {
                    retain_error(&mut progress, error, true);
                }
            }
        }
        if let Some(claim) = self
            .resources
            .as_ref()
            .and_then(|resources| resources.namespace.as_ref())
        {
            claim.quarantine();
        }
    }
    fn begin_closing(&self) -> Result<(), Quarantined> {
        if let Some(claim) = self
            .resources
            .as_ref()
            .and_then(|resources| resources.namespace.as_ref())
        {
            claim.begin_closing().map_err(|_| Quarantined)?;
        }
        Ok(())
    }
    fn drop_retentions(&mut self) -> Result<(), Quarantined> {
        while let Some(native) = self.resources_mut().native.pop_front() {
            let native = Armed::new(native);
            if !self.native_call(|| native.retirement_is_checked())? {
                self.quarantine();
                return Err(Quarantined);
            }
            self.native_call(|| drop(native.into_inner()))?;
            if self.is_quarantined() {
                return Err(Quarantined);
            }
        }
        while let Some(provider) = self.resources_mut().providers.pop_front() {
            let provider = Armed::new(provider);
            if !self.native_call(|| provider.retirement_is_checked())? {
                self.quarantine();
                return Err(Quarantined);
            }
            self.native_call(|| drop(provider.into_inner()))?;
            if self.is_quarantined() {
                return Err(Quarantined);
            }
        }
        Ok(())
    }
    fn commit(
        mut self,
        mut completion: Option<worker::Completion>,
    ) -> Result<Option<CopyProgress>, Quarantined> {
        let resources = self.resources_mut();
        if let Some(claim) = &resources.namespace {
            if claim.is_quarantined() {
                return Err(Quarantined);
            }
            resources.namespace_errors.extend(claim.take_diagnostics());
        }
        if !resources.namespace_errors.is_empty() {
            if let Some(status) = &resources.status {
                let mut progress = status.lock().unwrap_or_else(|error| error.into_inner());
                for error in resources.namespace_errors.drain(..) {
                    retain_error(&mut progress, error, true);
                }
            } else if let Some(completion) = &mut completion {
                completion.retain_errors(std::mem::take(&mut resources.namespace_errors));
            } else {
                // A missing host sink cannot silently turn a native refresh
                // failure into completion. Keep both diagnostics and authority.
                if let Some(claim) = &resources.namespace {
                    claim.quarantine();
                }
                return Err(Quarantined);
            }
        }
        if let Some(claim) = &resources.namespace
            && claim.complete_after_drop().is_err()
        {
            claim.quarantine();
            return Err(Quarantined);
        }
        let mut resources = self
            .resources
            .take()
            .expect("Copy authority remains sealed");
        drop(resources.mutation.take());
        drop(resources.real_io.take());
        let progress = completion.map(worker::Completion::finish);
        if let Some(progress) = &progress {
            resources
                .receipt
                .as_ref()
                .expect("Copy receipt remains owned")
                .finish(progress);
        }
        drop(resources.receipt.take());
        resources
            .finished
            .take()
            .expect("one strong Copy finished owner")
            .finish();
        drop(resources);
        Ok(progress)
    }
}
impl Drop for AuthoritySeal {
    fn drop(&mut self) {
        if self.resources.is_some() {
            self.quarantine();
        }
        if let Some(resources) = self.resources.take() {
            std::mem::forget(resources);
        }
    }
}

pub(super) struct Scope<T: CheckedRetirement> {
    payload: Option<T>,
    seal: Option<AuthoritySeal>,
    before_drop: fn(&mut T),
    take_registration: fn(&mut T) -> Option<Arc<crate::drag_drop::Retention>>,
    completion: Option<worker::Completion>,
    result: Option<CopyProgress>,
    quarantined: bool,
}
pub(super) type Owned<T> = WorkerOwned<Scope<T>>;

/// Structural sender rollback, permitted only before either receipt is visible.
/// It returns the original command without invoking its captures or destructors.
pub(super) fn rollback_submission(value: Owned<super::submission::Submission>) -> Command {
    let (mut scope, lease) = value.into_parts();
    let mut seal = scope.seal.take().expect("unpublished Copy remains sealed");
    let resources = seal
        .resources
        .take()
        .expect("unpublished Copy owns resources");
    assert!(
        !resources.published,
        "published Copy cannot use transport rollback"
    );
    let command = scope
        .payload
        .as_mut()
        .expect("unpublished Copy owns command")
        .take_command();
    drop(scope.payload.take());
    // The original command has been returned. All remaining values are transport
    // metadata; the unused lease returns its actual slot without a native queue.
    drop(resources);
    drop(lease);
    command
}

/// An unpublished native-drop stage with no capture owns pure metadata only.
pub(super) fn rollback_empty_reservation(value: Owned<()>) {
    let (mut scope, lease) = value.into_parts();
    let mut seal = scope
        .seal
        .take()
        .expect("unpublished reservation remains sealed");
    let resources = seal
        .resources
        .take()
        .expect("unpublished reservation owns resources");
    assert!(
        !resources.published
            && resources.native.is_empty()
            && resources.providers.is_empty()
            && resources.unknown.is_empty(),
        "only empty unpublished reservations support metadata rollback"
    );
    let _ = scope.payload.take();
    drop(resources);
    drop(lease);
}

/// Native accepted-drop fill preserves its existing seal and cancellation token.
pub(super) fn attach_reserved_input(value: Owned<()>, input: CopyInput) -> Owned<CopyInput> {
    let (mut scope, lease) = value.into_parts();
    let _ = scope.payload.take();
    lease.own(Scope {
        payload: Some(input),
        seal: scope.seal.take(),
        before_drop: |_| {},
        take_registration: |_| None,
        completion: scope.completion.take(),
        result: scope.result.take(),
        quarantined: scope.quarantined,
    })
}

/// Main-loop fill only moves known metadata/owners. No checked/native callback.
pub(super) fn convert_submission(value: Owned<super::submission::Submission>) -> Owned<CopyInput> {
    let (mut scope, lease) = value.into_parts();
    let command = scope
        .payload
        .as_mut()
        .expect("admitted Copy owns command")
        .take_command();
    // Detach the empty shell while its original authority is still armed here.
    drop(scope.payload.take());
    let input = match CopyInput::from_command(command) {
        Ok(input) => input,
        Err(command) => {
            std::mem::forget(command);
            panic!("guarded Copy contains a Copy command");
        }
    };
    lease.own(Scope {
        payload: Some(input),
        seal: scope.seal.take(),
        before_drop: |_| {},
        take_registration: |_| None,
        completion: scope.completion.take(),
        result: scope.result.take(),
        quarantined: scope.quarantined,
    })
}

/// Attach the same pending controls without invoking the generic native map.
pub(super) fn register_input(value: Owned<CopyInput>, registration: InputRegistration) -> Request {
    let (mut scope, lease) = value.into_parts();
    let command = scope.payload.take().expect("admitted Copy owns its input");
    lease.own(Scope {
        payload: Some(registration.attach(command)),
        seal: scope.seal.take(),
        before_drop: |_| {},
        take_registration: |_| None,
        completion: scope.completion.take(),
        result: scope.result.take(),
        quarantined: scope.quarantined,
    })
}
impl<T: CheckedRetirement> Scope<T> {
    pub(super) fn new(payload: T, resources: Resources) -> Self {
        Self::with_cleanup(payload, resources, |_| {})
    }
    fn with_cleanup(payload: T, resources: Resources, before_drop: fn(&mut T)) -> Self {
        Self {
            payload: Some(payload),
            seal: Some(AuthoritySeal::new(resources)),
            before_drop,
            take_registration: |_| None,
            completion: None,
            result: None,
            quarantined: false,
        }
    }
    pub(super) fn resources_mut(&mut self) -> &mut Resources {
        self.seal
            .as_mut()
            .expect("Copy authority remains sealed")
            .resources_mut()
    }
    fn quarantine(&mut self) {
        self.quarantined = true;
        if let Some(seal) = &self.seal {
            seal.quarantine();
        }
        if let Some(payload) = self.payload.take() {
            std::mem::forget(payload);
        }
        // Armed Drop only forgets the boxed resource owner.
        drop(self.seal.take());
    }
    fn drain(&mut self) -> Result<(), Quarantined> {
        if self.quarantined
            || self
                .seal
                .as_ref()
                .is_some_and(AuthoritySeal::is_quarantined)
        {
            self.quarantine();
            return Err(Quarantined);
        }
        let Some(payload) = self.payload.take() else {
            return Ok(());
        };
        let mut payload = Armed::new(payload);
        let mut seal = self.seal.take().expect("Copy payload owns its authority");
        if seal.begin_closing().is_err() {
            seal.quarantine();
            self.quarantined = true;
            return Err(Quarantined);
        }
        let result = (|| {
            if !seal.native_call(|| payload.retirement_is_checked())? {
                seal.quarantine();
                return Err(Quarantined);
            }
            let cleanup = seal.native_call(|| (self.before_drop)(&mut payload));
            if cleanup.is_err() || seal.is_quarantined() {
                seal.quarantine();
                return Err(Quarantined);
            }
            if let Some(registration) = (self.take_registration)(&mut payload) {
                seal.resources_mut().retain_checked_provider(Box::new(
                    crate::drag_drop::RegistrationOwner::new(registration),
                ));
            }
            if seal.native_call(|| drop(payload.into_inner())).is_err() || seal.is_quarantined() {
                seal.quarantine();
                return Err(Quarantined);
            }
            if seal.drop_retentions().is_err() {
                seal.quarantine();
                return Err(Quarantined);
            }
            self.result = seal.commit(self.completion.take())?;
            Ok(())
        })();
        self.quarantined = result.is_err();
        result
    }
}
impl Scope<worker::Job> {
    #[cfg(test)]
    pub(super) fn job(payload: worker::Job, resources: Resources) -> Self {
        let completion = payload.completion();
        let mut scope = Self::with_cleanup(payload, resources, worker::Job::finish_abandoned);
        scope.completion = Some(completion);
        scope.take_registration = worker::Job::take_drop_retention;
        scope
    }
}
impl<T: CheckedRetirement> Deref for Scope<T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.payload.as_ref().expect("Copy payload remains owned")
    }
}
impl<T: CheckedRetirement> DerefMut for Scope<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.payload.as_mut().expect("Copy payload remains owned")
    }
}
impl<T: CheckedRetirement> Drop for Scope<T> {
    fn drop(&mut self) {
        let _ = self.drain();
    }
}

/// Consuming transformations keep the armed seal outside the native closure.
pub(super) fn map<T: CheckedRetirement + 'static, U: CheckedRetirement + 'static>(
    value: Owned<T>,
    transform: impl FnOnce(T) -> U,
) -> Result<Owned<U>, Quarantined> {
    map_inner(value, transform, |_| {}, |_| None, |_| None)
}
fn map_inner<T: CheckedRetirement + 'static, U: CheckedRetirement + 'static>(
    value: Owned<T>,
    transform: impl FnOnce(T) -> U,
    before_drop: fn(&mut U),
    take_registration: fn(&mut U) -> Option<Arc<crate::drag_drop::Retention>>,
    completion: fn(&U) -> Option<worker::Completion>,
) -> Result<Owned<U>, Quarantined> {
    let (mut scope, lease) = value.into_parts();
    if scope.quarantined
        || scope
            .seal
            .as_ref()
            .is_some_and(AuthoritySeal::is_quarantined)
    {
        std::mem::forget(transform);
        scope.quarantine();
        return Err(Quarantined);
    }
    let qualified = scope
        .seal
        .as_ref()
        .expect("Copy authority remains sealed")
        .native_call(|| {
            scope
                .payload
                .as_ref()
                .expect("Copy payload remains owned")
                .retirement_is_checked()
        });
    if !matches!(qualified, Ok(true)) {
        std::mem::forget(transform);
        scope.quarantine();
        return Err(Quarantined);
    }
    let payload = Armed::new(scope.payload.take().expect("Copy payload remains owned"));
    let seal = scope.seal.take();
    let payload = match seal
        .as_ref()
        .expect("Copy authority remains sealed")
        .native_call(|| transform(payload.into_inner()))
    {
        Ok(payload) if !seal.as_ref().is_some_and(AuthoritySeal::is_quarantined) => payload,
        Ok(payload) => {
            std::mem::forget(payload);
            if let Some(seal) = &seal {
                seal.quarantine();
            }
            return Err(Quarantined);
        }
        Err(error) => {
            if let Some(seal) = &seal {
                seal.quarantine();
            }
            return Err(error);
        }
    };
    let completion = completion(&payload);
    Ok(lease.own(Scope {
        payload: Some(payload),
        seal,
        before_drop,
        take_registration,
        completion,
        result: None,
        quarantined: false,
    }))
}

/// Called inside the blocking task, before an unwind can become a JoinError.
pub(super) fn run<T: CheckedRetirement + 'static, R>(
    value: &mut Owned<T>,
    operation: impl FnOnce(&mut Scope<T>) -> R,
) -> Result<R, Quarantined> {
    if value.quarantined
        || value
            .seal
            .as_ref()
            .is_some_and(AuthoritySeal::is_quarantined)
    {
        std::mem::forget(operation);
        value.quarantine();
        return Err(Quarantined);
    }
    let qualified = value
        .seal
        .as_ref()
        .expect("Copy authority remains sealed")
        .native_call(|| {
            value
                .payload
                .as_ref()
                .expect("Copy payload remains owned")
                .retirement_is_checked()
        });
    if !matches!(qualified, Ok(true)) {
        std::mem::forget(operation);
        value.quarantine();
        return Err(Quarantined);
    }
    let context = match value
        .seal
        .as_ref()
        .expect("Copy authority remains sealed")
        .call_context()
    {
        Ok(context) => context,
        Err(error) => {
            std::mem::forget(operation);
            value.quarantine();
            return Err(error);
        }
    };
    let retirement = value
        .seal
        .as_ref()
        .expect("Copy authority remains sealed")
        .resources
        .as_ref()
        .expect("Copy resources remain sealed")
        .retirement
        .clone();
    let result = seal::checked(|| {
        retirement.with(|| match &context {
            Some(context) => context.invoke(|| operation(value)),
            None => Ok(operation(value)),
        })
    })
    .and_then(|result| result);
    drop(context);
    match result {
        Ok(result)
            if !value
                .seal
                .as_ref()
                .is_some_and(AuthoritySeal::is_quarantined) =>
        {
            Ok(result)
        }
        Ok(result) => {
            std::mem::forget(result);
            value.quarantine();
            Err(Quarantined)
        }
        Err(error) => {
            value.quarantine();
            Err(error)
        }
    }
}
pub(super) fn finish(job: Owned<worker::Job>) -> Result<CopyProgress, Quarantined> {
    let (mut scope, lease) = job.into_parts();
    scope.before_drop = worker::Job::finish_in_place;
    scope.drain()?;
    let progress = scope
        .result
        .take()
        .expect("confirmed Job retirement has a final result");
    drop(lease);
    Ok(progress)
}

/// A successful borrowed preparation may return native Job owners before its
/// retained preparation wrapper is destroyed. Arm that returned Job first;
/// failed destruction of the old payload must retain both ownership families.
pub(super) fn handoff_job<T: CheckedRetirement + 'static>(
    value: Owned<T>,
    job: worker::Job,
) -> Result<Owned<worker::Job>, Quarantined> {
    let job = Armed::new(job);
    let (mut scope, lease) = value.into_parts();
    if scope.quarantined
        || scope
            .seal
            .as_ref()
            .is_some_and(AuthoritySeal::is_quarantined)
    {
        scope.quarantine();
        return Err(Quarantined);
    }
    let mut old = Armed::new(scope.payload.take().expect("Copy payload remains owned"));
    let mut seal = scope.seal.take().expect("Copy authority remains sealed");
    let result = (|| {
        if !seal.native_call(|| old.retirement_is_checked())? {
            return Err(Quarantined);
        }
        seal.native_call(|| (scope.before_drop)(&mut old))?;
        if seal.is_quarantined() {
            return Err(Quarantined);
        }
        if let Some(registration) = (scope.take_registration)(&mut old) {
            seal.resources_mut().retain_checked_provider(Box::new(
                crate::drag_drop::RegistrationOwner::new(registration),
            ));
        }
        seal.native_call(|| drop(old.into_inner()))?;
        if seal.is_quarantined() {
            return Err(Quarantined);
        }
        Ok(())
    })();
    if result.is_err() {
        seal.quarantine();
        return Err(Quarantined);
    }
    if let Some(permit) = seal.resources_mut().real_io.clone() {
        // Link cleanup shares this SAME actual admitted permit. No new slot is
        // acquired, and no step can detach it before checked native destruction.
        seal.native_call(|| job.attach_io_permit(permit))?;
    }
    let completion = job.completion();
    Ok(lease.own(Scope {
        payload: Some(job.into_inner()),
        seal: Some(seal),
        before_drop: worker::Job::finish_abandoned,
        take_registration: worker::Job::take_drop_retention,
        completion: Some(completion),
        result: None,
        quarantined: false,
    }))
}

/// A retirement-domain receipt only acknowledges processing of its wrapper.
/// Runtime finalizers use this result to distinguish confirmed native close.
pub(super) fn retire_confirmed<T: CheckedRetirement + 'static>(
    value: Owned<T>,
) -> Result<(), Quarantined> {
    let (mut scope, lease) = value.into_parts();
    let result = scope.drain();
    drop(lease);
    result
}

#[cfg(test)]
#[path = "copy_seal_tests.rs"]
pub(super) mod seal_tests;
#[cfg(test)]
#[path = "copy_ownership_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "copy_nested_retirement_tests.rs"]
mod nested_retirement_tests;

// Aggregate predicates follow typed native provenance and explicit provider
// bridges. Neither geometry Unknown nor a generic Send bound supplies proof.
impl CheckedRetirement for worker::Job {
    fn retirement_is_checked(&self) -> bool {
        self.retirement_is_checked()
    }
}
impl CheckedRetirement for input::CopyInput {
    fn retirement_is_checked(&self) -> bool {
        self.retirement_is_checked()
    }
}
impl CheckedRetirement for preparation::Preparation {
    fn retirement_is_checked(&self) -> bool {
        self.retirement_is_checked()
    }
}
impl CheckedRetirement for Input {
    fn retirement_is_checked(&self) -> bool {
        self.command.retirement_is_checked()
    }
}

impl CheckedRetirement for crate::copy_namespace::NativeOwners {
    fn retirement_is_checked(&self) -> bool {
        self.retirement_is_checked()
    }
}

#[cfg(test)]
mod namespace_integration {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Payload {
        drops: Arc<AtomicUsize>,
        gate: Arc<midnight_work_budget::Gate>,
    }
    impl CheckedRetirement for Payload {
        fn retirement_is_checked(&self) -> bool {
            true
        }
    }
    impl Drop for Payload {
        fn drop(&mut self) {
            assert_eq!(self.gate.snapshot().active_shared, 1);
            assert!(
                midnight_platform_common::transfer_namespace::current_transfer_namespace_changes()
                    .is_some()
            );
            let result =
                midnight_platform_common::transfer_namespace::with_transfer_namespace_change(
                    &Cancellation::default(),
                    |_| Ok(()),
                );
            result.operation.unwrap();
            result.refresh.unwrap();
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }
    struct Fixture {
        gate: Arc<midnight_work_budget::Gate>,
        capacity: Arc<Semaphore>,
        mutation: Arc<Semaphore>,
        pending: crate::copy_finished::Predecessors,
        drops: Arc<AtomicUsize>,
        domain: crate::retirement::RetirementDomain,
    }
    fn fixture() -> (Fixture, Owned<Payload>) {
        let gate = Arc::new(midnight_work_budget::Gate::new(1));
        let capacity = Arc::new(Semaphore::new(1));
        let mutation = Arc::new(Semaphore::new(1));
        let mut finished = crate::copy_finished::Registry::default();
        let (owner, earlier) = finished.register().unwrap();
        let (_, pending) = finished.register().unwrap();
        let mut resources = Resources::admission(
            crate::copy_fence::CopyFence::default().register(),
            owner,
            capacity.clone().try_acquire_owned().unwrap(),
            earlier,
        );
        resources.mutation = Some(mutation.clone().try_acquire_owned().unwrap());
        let io = Arc::new(
            gate.acquire(midnight_work_budget::Admission::Shared, || false)
                .unwrap(),
        );
        resources.attach_io(gate.clone(), io.clone()).unwrap();
        resources.attach_status(Arc::new(Mutex::new(CopyProgress::queued(
            OperationId(1),
            "target".into(),
            1,
            TransferIntent::Copy,
        ))));
        let registry = crate::copy_namespace::Registry::new_with_io(gate.clone());
        let (claim, pins) = registry.new_job(1).unwrap();
        resources.install_namespace(claim, pins).unwrap();
        resources
            .namespace()
            .unwrap()
            .plan_union(
                vec![(TransferNamespaceAccess::Write, None)],
                io,
                &Cancellation::default(),
            )
            .unwrap();
        assert!(
            resources
                .namespace()
                .unwrap()
                .try_grant(&Cancellation::default())
                .unwrap()
        );
        let drops = Arc::new(AtomicUsize::new(0));
        let domain = crate::retirement::RetirementDomain::try_start(1, 0).unwrap();
        let owned = domain.try_reserve().unwrap().own(Scope::new(
            Payload {
                drops: drops.clone(),
                gate: gate.clone(),
            },
            resources,
        ));
        (
            Fixture {
                gate,
                capacity,
                mutation,
                pending,
                drops,
                domain,
            },
            owned,
        )
    }
    fn processed(owned: Owned<Payload>) {
        let retired = owned.retire();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(5), retired)
                .await
                .unwrap()
                .unwrap();
        });
    }
    #[test]
    fn sealed_drop_installs_closing_context_and_releases_real_slot_before_finished() {
        let (fixture, mut owned) = fixture();
        run(&mut owned, |_| {
            assert!(
                midnight_platform_common::transfer_namespace::current_transfer_namespace_changes()
                    .is_some()
            );
        })
        .unwrap();
        assert!(
            midnight_platform_common::transfer_namespace::current_transfer_namespace_changes()
                .is_none()
        );
        processed(owned);
        assert_eq!(fixture.drops.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.gate.snapshot().active_shared, 0);
        assert_eq!(fixture.mutation.available_permits(), 1);
        assert_eq!(fixture.capacity.available_permits(), 1);
        assert!(fixture.pending.is_finished());
        assert!(fixture.domain.try_reserve().is_ok());
    }
    #[test]
    fn unfinished_namespace_transaction_trips_seal_even_without_rust_unwind() {
        let (fixture, mut owned) = fixture();
        let result = run(&mut owned, |_| {
            drop(
                midnight_platform_common::transfer_namespace::begin_transfer_namespace_change(
                    &Cancellation::default(),
                )
                .unwrap(),
            );
        });
        assert_eq!(result, Err(Quarantined));
        assert!(
            midnight_platform_common::transfer_namespace::current_transfer_namespace_changes()
                .is_none()
        );
        processed(owned);
        assert_eq!(fixture.drops.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.gate.snapshot().active_shared, 1);
        assert_eq!(fixture.mutation.available_permits(), 0);
        assert_eq!(fixture.capacity.available_permits(), 0);
        assert!(!fixture.pending.is_finished());
    }
}
