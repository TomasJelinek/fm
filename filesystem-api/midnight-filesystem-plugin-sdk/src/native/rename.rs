//! Bounded protocol ledger; native pins/namespace implementations belong to plugins.
use super::*;
use state::{Charge, Slot};
use std::{
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    sync::atomic::{AtomicBool, Ordering},
};

/// Pessimistic retained allocation charge per opening/live Rename resource.
/// Together with 32 slots this reserves at most 32 MiB within the native budget.
/// Authors must keep opaque native pin/conflict/receipt capacities within this
/// reservation; the SDK cannot inspect allocations hidden behind a trait object.
pub const RENAME_RESOURCE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug)]
pub struct PreparedRenameInfo {
    pub source: RenameObject,
    pub parent_identity: Vec<u8>,
    pub recoverable_replace: bool,
}
/// A single source/parent pin, captured and binding-validated during Prepare.
/// Never replace that authority with a new pathname open. Private allocations
/// share RENAME_RESOURCE_BYTES, including the SDK ledger and captured paths.
pub trait NativeRename: Send + Sync + 'static {
    /// Return the immutable cached pin snapshot, without native I/O.
    fn prepared(&self) -> PreparedRenameInfo;
    /// The SDK has checked the exact destination and last conflict consent. The
    /// adapter must revalidate retained source/parent/target pins, reserve exact
    /// receipts before writes, and complete recovery after the commit gate even
    /// if canceled. Return Conflict, Unchanged, Renamed or RecoveryRequired only.
    /// Conflict.token is assigned by the SDK; do not use it as native authority.
    /// Ready/Executing or malformed receipts become terminal unknown outcomes.
    fn execute(
        &self,
        name: &NativeText,
        policy: &RenamePolicy,
        context: &CallContext<'_>,
    ) -> RenameOutcome;
}
struct Admission {
    mount: Arc<Mount>,
    _charge: Charge,
    // Last: native resources, mount ownership and allocation charges close first.
    _slot: Slot,
}
#[derive(PartialEq, Eq)]
struct Input {
    name: NativeText,
    policy: RenamePolicy,
}
struct Ledger {
    attempt: u64,
    input: Option<Arc<Input>>,
    outcome: RenameOutcome,
}
pub(super) struct RenameLease {
    // Native pin closes before mount/provider ownership and slot admission.
    native: Arc<dyn NativeRename>,
    kind: Kind,
    source_identity: Vec<u8>,
    replace: bool,
    ledger: Mutex<Ledger>,
    pub(super) busy: AtomicBool,
    pub(super) closed: AtomicBool,
    admission: Admission,
}
impl RenameLease {
    pub(super) fn current(&self) -> Result<(), Failure> {
        if self.closed.load(Ordering::Acquire) {
            return Err(failure(ErrorCode::StaleReference));
        }
        self.admission.mount.current()
    }
}
struct Busy(Option<Arc<RenameLease>>);
impl Drop for Busy {
    fn drop(&mut self) {
        if let Some(lease) = &self.0 {
            lease.busy.store(false, Ordering::Release);
        }
    }
}
fn unknown(message: &'static str) -> RenameOutcome {
    RenameOutcome::RecoveryRequired {
        source_location: RenameSourceLocation::Unknown,
        destination_name: None,
        recovery: vec![],
        error: Failure::new(ErrorCode::Io, message),
    }
}
fn object_capacity(value: &RenameObject) -> usize {
    value
        .identity
        .capacity()
        .saturating_add(value.revision.capacity())
        .saturating_add(value.name.bytes.capacity())
}
fn outcome_capacity(value: &RenameOutcome) -> usize {
    let recoveries = |values: &Vec<RenameRecovery>| {
        values.iter().fold(
            values
                .capacity()
                .saturating_mul(size_of::<RenameRecovery>()),
            |size, value| {
                size.saturating_add(value.address.bytes.capacity())
                    .saturating_add(value.identity.capacity())
            },
        )
    };
    match value {
        RenameOutcome::Ready | RenameOutcome::Executing => 0,
        RenameOutcome::Conflict(value) => object_capacity(&value.target),
        RenameOutcome::Unchanged(error) => error.message.capacity(),
        RenameOutcome::Renamed { name, recovery, .. } => {
            name.bytes.capacity().saturating_add(recoveries(recovery))
        }
        RenameOutcome::RecoveryRequired {
            destination_name,
            recovery,
            error,
            ..
        } => destination_name
            .as_ref()
            .map_or(0, |name| name.bytes.capacity())
            .saturating_add(recoveries(recovery))
            .saturating_add(error.message.capacity()),
    }
}
fn same_kind(listed: Kind, pinned: Kind) -> bool {
    listed == pinned
        || matches!(
            (listed, pinned),
            (Kind::DirectoryLink, Kind::Symlink) | (Kind::Symlink, Kind::DirectoryLink)
        )
}
impl<A: NativeAdapter> NativeFilesystem<A> {
    pub(super) fn prepare_rename(
        &self,
        mount: u64,
        node: u64,
        context: &CallContext<'_>,
    ) -> Result<Response, Failure> {
        let (mount, node) = self.node(mount, node)?;
        if node.kind == Kind::Other {
            return Err(failure(ErrorCode::Unsupported));
        }
        let basename = node
            .path
            .file_name()
            .ok_or_else(|| failure(ErrorCode::Unsupported))?;
        let slot = self.slots.rename()?;
        let mut charge = Charge::new(self.budget.clone());
        charge.grow(RENAME_RESOURCE_BYTES)?;
        let admission = Admission {
            mount,
            _charge: charge,
            _slot: slot,
        };
        let (encoding, bytes) = A::name(basename);
        let admitted_name = NativeText { encoding, bytes };
        admitted_name
            .validate_rename_observed_basename()
            .map_err(|_| failure(ErrorCode::Invalid))?;
        if node
            .path
            .capacity()
            .saturating_add(admitted_name.bytes.capacity())
            .saturating_add(MAX_RENAME_RECEIPT_BYTES)
            > RENAME_RESOURCE_BYTES
        {
            return Err(failure(ErrorCode::ResourceLimit));
        }
        let handle = crate::allocate_handle()?;
        {
            // Reserve all possible live insertions before opening native pins.
            let mut state = lock(&self.state);
            state
                .renames
                .try_reserve(32)
                .map_err(|_| failure(ErrorCode::ResourceLimit))?;
        }
        admission.mount.current()?;
        if context.is_cancelled() {
            return Err(failure(ErrorCode::Cancelled));
        }
        // The first authoritative object identity is captured here, once. A
        // same-kind replacement since Browse may correctly be this current pin.
        let native = A::prepare_rename(&node.path, context)?;
        let info = native.prepared();
        if !same_kind(node.kind, info.source.kind) || info.source.name != admitted_name {
            return Err(failure(ErrorCode::SourceChanged));
        }
        let prepared = PreparedRename {
            handle,
            source: info.source,
            parent_identity: info.parent_identity,
            recoverable_replace: info.recoverable_replace,
        };
        prepared
            .validate()
            .map_err(|_| failure(ErrorCode::Invalid))?;
        if object_capacity(&prepared.source)
            .saturating_add(prepared.parent_identity.capacity())
            .saturating_add(MAX_RENAME_RECEIPT_BYTES)
            > RENAME_RESOURCE_BYTES
        {
            return Err(failure(ErrorCode::ResourceLimit));
        }
        let lease = Arc::new(RenameLease {
            native,
            kind: prepared.source.kind,
            source_identity: prepared.source.identity.clone(),
            replace: prepared.recoverable_replace,
            ledger: Mutex::new(Ledger {
                attempt: 0,
                input: None,
                outcome: RenameOutcome::Ready,
            }),
            busy: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            admission,
        });
        {
            let mut state = lock(&self.state);
            lease.current()?;
            state.renames.insert(handle, lease);
        }
        Ok(Response::RenamePrepared(prepared))
    }
    fn rename_lease(&self, id: u64) -> Result<Arc<RenameLease>, Failure> {
        lock(&self.state)
            .renames
            .get(&id)
            .cloned()
            .ok_or_else(|| failure(ErrorCode::StaleReference))
    }
    pub(super) fn get_rename_state(&self, id: u64) -> Result<Response, Failure> {
        let lease = self.rename_lease(id)?;
        if lease.closed.load(Ordering::Acquire) {
            return Err(failure(ErrorCode::StaleReference));
        }
        let ledger = lock(&lease.ledger);
        let value = RenameState {
            prepared: id,
            attempt: ledger.attempt,
            outcome: ledger.outcome.clone(),
        };
        if lease.closed.load(Ordering::Acquire) {
            return Err(failure(ErrorCode::StaleReference));
        }
        Ok(Response::RenameState(value))
    }
    pub(super) fn close_rename(&self, id: u64) -> Result<Response, Failure> {
        let retired = {
            let mut state = lock(&self.state);
            let lease = state
                .renames
                .get(&id)
                .ok_or_else(|| failure(ErrorCode::StaleReference))?;
            if lease.busy.load(Ordering::Acquire) {
                return Err(failure(ErrorCode::Busy));
            }
            lease.closed.store(true, Ordering::Release);
            state.renames.remove(&id)
        };
        drop(retired); // Native disposal is outside registry locks.
        Ok(Response::Unit)
    }
    pub(super) fn rename(
        &self,
        id: u64,
        attempt: u64,
        name: NativeText,
        policy: RenamePolicy,
        context: &CallContext<'_>,
    ) -> Result<Reply, Failure> {
        let lease = {
            let state = lock(&self.state);
            let lease = state
                .renames
                .get(&id)
                .ok_or_else(|| failure(ErrorCode::StaleReference))?;
            if lease.closed.load(Ordering::Acquire) {
                return Err(failure(ErrorCode::StaleReference));
            }
            lease
                .busy
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .map_err(|_| failure(ErrorCode::Busy))?;
            lease.clone()
        };
        let mut busy = Busy(Some(lease.clone()));
        let value = self.rename_attempt(&lease, id, attempt, name, policy, context)?;
        let held = busy.0.take().ok_or_else(|| failure(ErrorCode::Invalid))?;
        Ok(Reply::owned(Response::RenameState(value), move |_| {
            // Delivery failure releases this call only; the known prepared
            // resource and its truthful result remain available for query.
            held.busy.store(false, Ordering::Release);
            Ok(())
        }))
    }
    fn rename_attempt(
        &self,
        lease: &RenameLease,
        id: u64,
        attempt: u64,
        name: NativeText,
        policy: RenamePolicy,
        context: &CallContext<'_>,
    ) -> Result<RenameState, Failure> {
        if attempt == 0 {
            return Err(failure(ErrorCode::Invalid));
        }
        name.validate_rename_basename()
            .map_err(|_| failure(ErrorCode::Invalid))?;
        let input = Arc::new(Input { name, policy });
        let mut ledger = lock(&lease.ledger);
        if attempt < ledger.attempt {
            return Err(failure(ErrorCode::StaleReference));
        }
        if attempt == ledger.attempt {
            if ledger.input.as_deref() != Some(input.as_ref()) {
                return Err(failure(ErrorCode::Invalid));
            }
            return Ok(RenameState {
                prepared: id,
                attempt,
                outcome: ledger.outcome.clone(),
            });
        }
        if ledger.attempt != 0 && !matches!(ledger.outcome, RenameOutcome::Conflict(_)) {
            return Err(failure(ErrorCode::StaleReference));
        }
        if ledger.attempt.checked_add(1) != Some(attempt) {
            return Err(failure(ErrorCode::Invalid));
        }
        match &input.policy {
            RenamePolicy::NoReplace => {}
            RenamePolicy::RecoverableCurrentEntry { conflict } => {
                let RenameOutcome::Conflict(previous) = &ledger.outcome else {
                    return Err(failure(ErrorCode::Invalid));
                };
                if !previous.can_replace
                    || !lease.replace
                    || lease.kind != Kind::File
                    || previous.token != *conflict
                    || ledger
                        .input
                        .as_ref()
                        .is_none_or(|previous| previous.name != input.name)
                {
                    return Err(failure(ErrorCode::Invalid));
                }
            }
        }
        if input
            .name
            .bytes
            .capacity()
            .saturating_add(MAX_RENAME_RECEIPT_BYTES)
            .saturating_add(size_of::<RenameLease>())
            > RENAME_RESOURCE_BYTES
            || context.config().reply_bytes < MAX_RENAME_RECEIPT_BYTES as u64
        {
            return Err(failure(ErrorCode::ResourceLimit));
        }
        // Nonwrapping conflict identity and receipt admission precede native work.
        let conflict_token = crate::allocate_handle()?;
        let retired_input = ledger.input.replace(input.clone());
        let retired_outcome = std::mem::replace(&mut ledger.outcome, RenameOutcome::Executing);
        ledger.attempt = attempt;
        drop(ledger);
        drop(retired_input);
        drop(retired_outcome);
        let outcome = if context.is_cancelled() {
            RenameOutcome::Unchanged(failure(ErrorCode::Cancelled))
        } else {
            let executed = catch_unwind(AssertUnwindSafe(|| {
                lease.native.execute(&input.name, &input.policy, context)
            }));
            // The interface cannot observe transient staging/restoration. Retire
            // this mount's browse epoch conservatively after every native attempt.
            lease.admission.mount.invalidate();
            match executed {
                Ok(value) => value,
                Err(payload) => {
                    lock(&lease.ledger).outcome = unknown(
                        "Rename provider panicked; namespace outcome could not be confirmed.",
                    );
                    resume_unwind(payload)
                }
            }
        };
        let mut outcome = outcome;
        if let RenameOutcome::Conflict(value) = &mut outcome {
            value.token = conflict_token;
            value.can_replace &= lease.replace
                && lease.kind == Kind::File
                && value.target.kind == Kind::File
                && value.target.identity != lease.source_identity;
        }
        if matches!(outcome, RenameOutcome::Ready | RenameOutcome::Executing)
            || outcome.validate().is_err()
            || outcome_capacity(&outcome)
                .saturating_add(input.name.bytes.capacity())
                .saturating_add(size_of::<RenameLease>())
                > RENAME_RESOURCE_BYTES
        {
            outcome = unknown(
                "Rename provider returned an invalid receipt; namespace outcome could not be confirmed.",
            );
        }
        let mut ledger = lock(&lease.ledger);
        ledger.outcome = outcome; // Record before response cloning/encoding/publication.
        Ok(RenameState {
            prepared: id,
            attempt,
            outcome: ledger.outcome.clone(),
        })
    }
}
