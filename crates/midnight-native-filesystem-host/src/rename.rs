//! Owned prepared Rename receipts. All calls and final disposal belong on backend workers.
use crate::{
    Cancel, NativeProvider,
    client::{Lease, check_cancel, validate_node},
    discovery::invalid,
    paths,
    services::{entry, fs_error},
};
use midnight_core as core;
use midnight_filesystem_plugin_api as api;
use std::{
    ffi::OsString,
    num::NonZeroU64,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value.lock().unwrap_or_else(|error| error.into_inner())
}

fn bounded(error: api::Failure) -> api::Failure {
    if error.message.len() > api::MAX_DIAGNOSTIC {
        invalid("Rename failure diagnostic exceeds the protocol bound")
    } else {
        error
    }
}

fn call(
    owner: &Owned,
    request: &api::Request,
    cancelled: Cancel<'_>,
) -> Result<api::Response, api::Failure> {
    let result = match catch_unwind(AssertUnwindSafe(|| {
        owner.provider.transport.call(request, cancelled)
    })) {
        Ok(result) => result.map_err(bounded),
        Err(_) => {
            lock(&owner.resources).publication_unknown = true;
            lock(&owner.ledger).blocked = true;
            return Err(api::Failure::new(
                api::ErrorCode::Io,
                "Native Rename transport panicked; its result is unconfirmed",
            ));
        }
    };
    if result
        .as_ref()
        .is_err_and(|error| error.code == api::ErrorCode::Invalid)
    {
        // The FFI boundary can hide an unknown owned wrong-class reply when it
        // poisons. A later receipt query does not prove that extra owner absent.
        lock(&owner.resources).publication_unknown = true;
        lock(&owner.ledger).blocked = true;
    }
    result
}

fn observation() -> Result<core::RenameConflictToken, api::Failure> {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let value = NEXT
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .map_err(|_| invalid("Host Rename observation identifiers are exhausted"))?;
    Ok(core::RenameConflictToken(
        NonZeroU64::new(value).expect("observation identifiers begin at one"),
    ))
}

fn terminal(outcome: &core::RenameOutcome) -> bool {
    matches!(
        outcome,
        core::RenameOutcome::Unchanged(_)
            | core::RenameOutcome::Renamed { .. }
            | core::RenameOutcome::RecoveryRequired { .. }
            | core::RenameOutcome::Unconfirmed { .. }
    )
}

fn known_terminal(outcome: &core::RenameOutcome) -> bool {
    matches!(
        outcome,
        core::RenameOutcome::Unchanged(_)
            | core::RenameOutcome::Renamed { .. }
            | core::RenameOutcome::RecoveryRequired { .. }
    )
}

struct Handle {
    value: u64,
    closed: bool,
}

#[derive(Default)]
struct Resources {
    mount: Option<Lease>,
    handles: Vec<Handle>,
    unexpected: Vec<Lease>,
    publication_unknown: bool,
    failure: Option<core::FsError>,
    closed: bool,
}

struct Input {
    value: core::RenameAttempt,
    name: api::NativeText,
    policy: api::RenamePolicy,
    reserved: core::RenameConflictToken,
    previous: Option<(u64, api::RenameConflict)>,
}

struct Conflict {
    token: core::RenameConflictToken,
    raw: api::RenameConflict,
    attempt: u64,
    name: OsString,
}

struct Ledger {
    input: Option<Arc<Input>>,
    state: core::RenameState,
    known_attempt: u64,
    recovery: Vec<core::RenameRecovery>,
    conflict: Option<Conflict>,
    active: bool,
    handler: bool,
    blocked: bool,
    stopping: bool,
}

struct Owned {
    provider: NativeProvider,
    path: PathBuf,
    resolved: OnceLock<api::Node>,
    prepared: OnceLock<api::PreparedRename>,
    item: OnceLock<core::MutationItem>,
    resources: Mutex<Resources>,
    ledger: Mutex<Ledger>,
    // Queries and CloseRename share the provider's one reserved control lane.
    control: Mutex<()>,
    // Serializes finish callers, without blocking completion of a Rename callback.
    finishing: Mutex<()>,
}

/// The box exists before resource-producing dispatch. A nonclosed final Drop
/// quarantines this complete owned component, including its mount and provider.
struct Resource {
    owned: Option<Box<Owned>>,
}

impl Resource {
    fn new(provider: &NativeProvider, path: &Path) -> Self {
        Self {
            owned: Some(Box::new(Owned {
                provider: provider.clone(),
                path: path.to_owned(),
                resolved: OnceLock::new(),
                prepared: OnceLock::new(),
                item: OnceLock::new(),
                resources: Mutex::new(Resources::default()),
                ledger: Mutex::new(Ledger {
                    input: None,
                    state: core::RenameState {
                        attempt: 0,
                        outcome: core::RenameOutcome::Ready,
                    },
                    known_attempt: 0,
                    recovery: Vec::new(),
                    conflict: None,
                    active: false,
                    handler: false,
                    blocked: false,
                    stopping: false,
                }),
                control: Mutex::new(()),
                finishing: Mutex::new(()),
            })),
        }
    }

    fn owned(&self) -> &Owned {
        self.owned.as_deref().expect("live Rename resource owner")
    }

    fn failure(self, error: api::Failure) -> core::RenamePrepareFailure {
        let error = self.owned().error("prepare rename", error);
        let retained = if matches!(
            core::RenameResource::finish(&self),
            core::RenameFinish::Closed
        ) {
            None
        } else {
            Some(Arc::new(self) as Arc<dyn core::RenameResource>)
        };
        core::RenamePrepareFailure { error, retained }
    }
}

impl core::RenameResource for Resource {
    fn finish(&self) -> core::RenameFinish {
        self.owned().finish()
    }
}

impl core::PreparedRename for Resource {
    fn item(&self) -> &core::MutationItem {
        self.owned()
            .item
            .get()
            .expect("only validated preparations expose an item")
    }

    fn attempt(
        &self,
        input: &core::RenameAttempt,
        cancel: &core::Cancellation,
    ) -> core::RenameState {
        self.owned().attempt(input, cancel)
    }

    fn query(&self) -> core::RenameState {
        self.owned().query(None)
    }
}

impl Drop for Resource {
    fn drop(&mut self) {
        let Some(owned) = self.owned.take() else {
            return;
        };
        let closed = catch_unwind(AssertUnwindSafe(|| owned.finish()))
            .is_ok_and(|result| matches!(result, core::RenameFinish::Closed));
        if !closed {
            // No handle-only detached cleanup: keep the provider, mount, pins,
            // cached facts and ledger together. Task4C retains its outer chain.
            std::mem::forget(owned);
        }
    }
}

impl core::RenameService for NativeProvider {
    fn prepare(
        &self,
        path: &Path,
        cancel: &core::Cancellation,
    ) -> Result<Arc<dyn core::PreparedRename>, core::RenamePrepareFailure> {
        let before = || -> Result<api::Address, api::Failure> {
            check_cancel(&|| cancel.is_cancelled())?;
            self.require(api::abi::CAP_NATIVE_ADDRESS | api::abi::CAP_PREPARED_RENAME)?;
            let address = paths::encode(path)?;
            if address.bytes.len() > api::DEFAULT_REPLY_BYTES as usize {
                return Err(api::Failure::new(
                    api::ErrorCode::ResourceLimit,
                    "Native Rename locator exceeds the Resolve reply budget",
                ));
            }
            Ok(address)
        };
        let address = before().map_err(|error| core::RenamePrepareFailure {
            error: fs_error("prepare rename", path, bounded(error)),
            retained: None,
        })?;
        let resource = Resource::new(self, path);
        if let Err(error) = resource.owned().prepare(address, cancel) {
            return Err(resource.failure(error));
        }
        Ok(Arc::new(resource))
    }
}

impl Owned {
    fn error(&self, operation: &'static str, error: api::Failure) -> core::FsError {
        fs_error(operation, &self.path, bounded(error))
    }

    fn produce(
        &self,
        request: &api::Request,
        cancelled: Cancel<'_>,
    ) -> Result<api::Response, api::Failure> {
        lock(&self.resources).publication_unknown = true;
        let response = call(self, request, cancelled);
        if response
            .as_ref()
            .is_err_and(|error| error.code == api::ErrorCode::Cancelled)
        {
            // Private Transport contract: raw Cancelled is admission rejection
            // before the native callback. Provider cancellation is encoded Failed.
            lock(&self.resources).publication_unknown = false;
        }
        response
    }

    fn prepare(
        &self,
        address: api::Address,
        cancel: &core::Cancellation,
    ) -> Result<(), api::Failure> {
        check_cancel(&|| cancel.is_cancelled())?;
        let response = self.produce(
            &api::Request::Resolve {
                address: address.clone(),
            },
            &|| cancel.is_cancelled(),
        )?;
        let (mount, node) = match response {
            api::Response::Resolved { mount, node } if mount != 0 => {
                let mut resources = lock(&self.resources);
                resources.mount = Some(Lease::new(
                    &self.provider,
                    api::Request::CloseMount { mount },
                ));
                resources.publication_unknown = false;
                (mount, node)
            }
            api::Response::Failed(error) if error.message.len() <= api::MAX_DIAGNOSTIC => {
                lock(&self.resources).publication_unknown = false;
                return Err(error);
            }
            response => {
                self.unexpected(&response);
                return Err(invalid(
                    "Native Rename Resolve returned an invalid response",
                ));
            }
        };
        check_cancel(&|| cancel.is_cancelled())?;
        validate_resolved(&node, &address)?;
        let node_id = node.id;
        self.resolved
            .set(node)
            .map_err(|_| invalid("Native Rename Resolve was already installed"))?;
        check_cancel(&|| cancel.is_cancelled())?;
        let response = self.produce(
            &api::Request::PrepareRename {
                mount,
                node: node_id,
            },
            &|| cancel.is_cancelled(),
        )?;
        let prepared = match response {
            api::Response::RenamePrepared(prepared) if prepared.handle != 0 => {
                let mut resources = lock(&self.resources);
                resources.handles.push(Handle {
                    value: prepared.handle,
                    closed: false,
                });
                resources.publication_unknown = false;
                prepared
            }
            api::Response::Failed(error) if error.message.len() <= api::MAX_DIAGNOSTIC => {
                lock(&self.resources).publication_unknown = false;
                return Err(error);
            }
            response => {
                self.unexpected(&response);
                return Err(invalid(
                    "Native Rename preparation returned an invalid response",
                ));
            }
        };
        // The valid disclosed handle is already owned before either check.
        check_cancel(&|| cancel.is_cancelled())?;
        prepared
            .validate()
            .map_err(|_| invalid("Invalid native prepared Rename snapshot"))?;
        let node = self
            .resolved
            .get()
            .expect("validated Resolve precedes Prepare");
        if prepared.source.name.encoding != node.name_encoding
            || prepared.source.name.bytes != node.raw_name
            || !same_kind(prepared.source.kind, node.kind)
        {
            return Err(invalid(
                "Prepared Rename source differs from the resolved entry",
            ));
        }
        let item = object_item(node, &prepared.source, &self.path)?;
        self.prepared
            .set(prepared)
            .map_err(|_| invalid("Native Rename preparation was already installed"))?;
        self.item
            .set(item)
            .map_err(|_| invalid("Native Rename preview was already installed"))?;
        check_cancel(&|| cancel.is_cancelled())
    }

    fn unexpected(&self, response: &api::Response) {
        let mut resources = lock(&self.resources);
        resources.publication_unknown = true;
        match response {
            api::Response::RenamePrepared(prepared) if prepared.handle != 0 => {
                if !resources
                    .handles
                    .iter()
                    .any(|handle| handle.value == prepared.handle)
                {
                    resources.handles.push(Handle {
                        value: prepared.handle,
                        closed: false,
                    });
                }
            }
            api::Response::Resolved { mount, .. } if *mount != 0 => {
                resources.unexpected.push(Lease::new(
                    &self.provider,
                    api::Request::CloseMount { mount: *mount },
                ));
            }
            api::Response::Opened { reader, .. } if *reader != 0 => {
                resources.unexpected.push(Lease::new(
                    &self.provider,
                    api::Request::CloseRead { reader: *reader },
                ));
            }
            api::Response::Page { cursor, .. } if *cursor != 0 => {
                resources.unexpected.push(Lease::new(
                    &self.provider,
                    api::Request::CloseList { cursor: *cursor },
                ));
            }
            _ => {}
        }
        drop(resources);
        lock(&self.ledger).blocked = true;
    }

    fn refused(&self, attempt: u64, error: api::Failure) -> core::RenameState {
        core::RenameState {
            attempt,
            outcome: core::RenameOutcome::Unchanged(self.error("rename", error)),
        }
    }

    fn attempt(
        &self,
        input: &core::RenameAttempt,
        cancel: &core::Cancellation,
    ) -> core::RenameState {
        let mut ledger = lock(&self.ledger);
        if terminal(&ledger.state.outcome) || ledger.handler || ledger.blocked {
            return ledger.state.clone();
        }
        if let Some(previous) = &ledger.input
            && previous.value.number == input.number
        {
            return if previous.value == *input {
                ledger.state.clone()
            } else {
                self.refused(
                    input.number.get(),
                    invalid("Rename replay changed its payload"),
                )
            };
        }
        if ledger.stopping {
            return self.refused(input.number.get(), invalid("Rename resource is finishing"));
        }
        let next = ledger.input.as_ref().map_or(Some(1), |previous| {
            previous.value.number.get().checked_add(1)
        });
        let first = ledger.input.is_none();
        if next != Some(input.number.get())
            || (!first && !matches!(ledger.state.outcome, core::RenameOutcome::Conflict(_)))
        {
            return self.refused(
                input.number.get(),
                invalid("Invalid Rename attempt sequence"),
            );
        }
        if let Err(error) = check_cancel(&|| cancel.is_cancelled()) {
            return self.refused(input.number.get(), error);
        }
        let name = match paths::rename_name(&input.name) {
            Ok(name) => name,
            Err(error) => return self.refused(input.number.get(), error),
        };
        let policy = match input.policy {
            core::RenamePolicy::NoReplace => api::RenamePolicy::NoReplace,
            core::RenamePolicy::RecoverableCurrentEntry { conflict } => {
                let Some(observed) = &ledger.conflict else {
                    return self.refused(input.number.get(), invalid("No current Rename consent"));
                };
                if observed.token != conflict
                    || observed.name != input.name
                    || observed.attempt.checked_add(1) != Some(input.number.get())
                    || !observed.raw.can_replace
                {
                    return self.refused(
                        input.number.get(),
                        invalid("Rename consent does not own this input"),
                    );
                }
                api::RenamePolicy::RecoverableCurrentEntry {
                    conflict: observed.raw.token,
                }
            }
        };
        let reserved = match observation() {
            Ok(value) => value,
            Err(error) => return self.refused(input.number.get(), error),
        };
        let previous = ledger
            .conflict
            .as_ref()
            .map(|value| (value.attempt, value.raw.clone()));
        let input = Arc::new(Input {
            value: input.clone(),
            name,
            policy,
            reserved,
            previous,
        });
        ledger.input = Some(input.clone());
        ledger.conflict = None;
        ledger.state = core::RenameState {
            attempt: input.value.number.get(),
            outcome: core::RenameOutcome::Executing,
        };
        ledger.active = true;
        ledger.handler = true;
        drop(ledger);
        let _handler = Handler(self);
        let prepared = self.prepared.get().expect("usable prepared Rename").handle;
        let response = call(
            self,
            &api::Request::Rename {
                prepared,
                attempt: input.value.number.get(),
                name: input.name.clone(),
                policy: input.policy.clone(),
            },
            &|| cancel.is_cancelled(),
        );
        // A return (including a caught Rust panic) proves this host dispatch ended.
        lock(&self.ledger).active = false;
        let response = match response {
            Ok(api::Response::RenameState(state)) => {
                let salvage = recovery_of(self, &state);
                match self.accept(state, None) {
                    Ok(state) => return state,
                    Err(error) => {
                        self.retain_recovery(salvage);
                        error
                    }
                }
            }
            Ok(api::Response::Failed(error)) => bounded(error),
            Ok(response) => {
                self.unexpected(&response);
                invalid("Native Rename returned an unexpected response")
            }
            Err(error) => error,
        };
        if known_terminal(&lock(&self.ledger).state.outcome) {
            return lock(&self.ledger).state.clone();
        }
        // Never resend operation21. Query only after the actual call returned.
        self.query(Some(self.error("rename", response)))
    }

    fn query(&self, original: Option<core::FsError>) -> core::RenameState {
        let _control = lock(&self.control);
        let context = {
            let ledger = lock(&self.ledger);
            let associated_recovery = original.is_some() && ledger.handler;
            if terminal(&ledger.state.outcome)
                && !(associated_recovery
                    && matches!(
                        ledger.state.outcome,
                        core::RenameOutcome::Unconfirmed { .. }
                    ))
            {
                return ledger.state.clone();
            }
            Query {
                active: ledger.active,
                input: ledger
                    .input
                    .as_ref()
                    .map_or(0, |input| input.value.number.get()),
                original,
            }
        };
        let prepared = self.prepared.get().expect("usable prepared Rename").handle;
        if lock(&self.resources)
            .handles
            .iter()
            .any(|handle| handle.value == prepared && handle.closed)
        {
            return lock(&self.ledger).state.clone();
        }
        let response = call(self, &api::Request::GetRenameState { prepared }, &|| false);
        let failure = match response {
            Ok(api::Response::RenameState(state)) => {
                let salvage = recovery_of(self, &state);
                match self.accept(state, Some(context)) {
                    Ok(state) => return state,
                    Err(error) => return self.unconfirmed(error, salvage),
                }
            }
            Ok(api::Response::Failed(error)) => bounded(error),
            Ok(response) => {
                self.unexpected(&response);
                invalid("Native Rename query returned an unexpected response")
            }
            Err(error) => error,
        };
        self.unconfirmed(failure, None)
    }

    fn unconfirmed(
        &self,
        error: api::Failure,
        salvage: Option<RecoveryBatch>,
    ) -> core::RenameState {
        let mut ledger = lock(&self.ledger);
        if known_terminal(&ledger.state.outcome) {
            return ledger.state.clone();
        }
        merge_associated_recovery(&mut ledger, salvage);
        ledger.blocked = true;
        let requested_name = ledger
            .input
            .as_ref()
            .map_or_else(OsString::new, |input| input.value.name.clone());
        ledger.state = core::RenameState {
            attempt: ledger
                .input
                .as_ref()
                .map_or(0, |input| input.value.number.get()),
            outcome: core::RenameOutcome::Unconfirmed {
                requested_name,
                last_known_attempt: ledger.known_attempt,
                recovery: ledger.recovery.clone(),
                error: self.error("query rename", error),
            },
        };
        ledger.conflict = None;
        ledger.state.clone()
    }

    fn retain_recovery(&self, salvage: Option<RecoveryBatch>) {
        let mut ledger = lock(&self.ledger);
        if !terminal(&ledger.state.outcome) {
            merge_associated_recovery(&mut ledger, salvage);
        }
    }

    fn accept(
        &self,
        state: api::RenameState,
        query: Option<Query>,
    ) -> Result<core::RenameState, api::Failure> {
        state
            .validate()
            .map_err(|_| invalid("Malformed native Rename receipt"))?;
        let prepared = self.prepared.get().expect("validated native Rename");
        if state.prepared != prepared.handle {
            return Err(invalid("Rename receipt belongs to another preparation"));
        }
        let mut ledger = lock(&self.ledger);
        if known_terminal(&ledger.state.outcome) {
            return Ok(ledger.state.clone());
        }
        let associated =
            ledger.handler && query.as_ref().is_none_or(|query| query.original.is_some());
        if matches!(
            ledger.state.outcome,
            core::RenameOutcome::Unconfirmed { .. }
        ) && !associated
        {
            return Ok(ledger.state.clone());
        }
        let Some(input) = ledger.input.clone() else {
            if state.attempt == 0 && matches!(state.outcome, api::RenameOutcome::Ready) {
                return Ok(ledger.state.clone());
            }
            return Err(invalid("Rename receipt has no admitted host input"));
        };
        let number = input.value.number.get();
        if state.attempt < number {
            let Some(query) = &query else {
                return Err(invalid("Rename dispatch returned an old receipt"));
            };
            if query.active || ledger.active || query.input < number {
                return Ok(ledger.state.clone());
            }
            let unadmitted = (state.attempt == 0
                && input.previous.is_none()
                && matches!(state.outcome, api::RenameOutcome::Ready))
                || input.previous.as_ref().is_some_and(|(attempt, previous)| {
                    *attempt == state.attempt
                        && matches!(&state.outcome, api::RenameOutcome::Conflict(current) if current == previous)
                });
            if unadmitted && let Some(error) = &query.original {
                ledger.state = core::RenameState {
                    attempt: number,
                    outcome: core::RenameOutcome::Unchanged(error.clone()),
                };
                ledger.conflict = None;
                return Ok(ledger.state.clone());
            }
            return Err(invalid("Rename query rewound its recorded attempt"));
        }
        if state.attempt != number {
            return Err(invalid("Rename receipt has an unexpected attempt number"));
        }
        if matches!(state.outcome, api::RenameOutcome::Executing)
            && matches!(
                ledger.state.outcome,
                core::RenameOutcome::Conflict(_) | core::RenameOutcome::Unconfirmed { .. }
            )
        {
            // A delayed Executing observation cannot revoke accepted consent or
            // resolve a provisional query fault. The associated actual result
            // may still supply authoritative terminal facts below.
            return Ok(ledger.state.clone());
        }
        let outcome = match state.outcome {
            api::RenameOutcome::Ready => return Err(invalid("Admitted Rename cannot be Ready")),
            api::RenameOutcome::Executing => core::RenameOutcome::Executing,
            api::RenameOutcome::Conflict(conflict) => {
                if conflict.can_replace
                    && (!prepared.recoverable_replace
                        || prepared.source.kind != api::Kind::File
                        || conflict.target.kind != api::Kind::File
                        || conflict.target.identity == prepared.source.identity)
                {
                    return Err(invalid(
                        "Native Rename conflict is not qualified for replacement",
                    ));
                }
                let observed = observed_name(&conflict.target.name)?;
                let parent = self
                    .path
                    .parent()
                    .ok_or_else(|| invalid("Rename source has no parent"))?;
                let target = object_item(
                    self.resolved.get().expect("validated Resolve"),
                    &conflict.target,
                    &parent.join(observed),
                )?;
                let token = if let Some(previous) = &ledger.conflict {
                    if previous.attempt == number && previous.raw == conflict {
                        previous.token
                    } else if previous.raw.token == conflict.token {
                        return Err(invalid("Rename conflict token changed its observation"));
                    } else {
                        observation()?
                    }
                } else {
                    input.reserved
                };
                let outcome = core::RenameOutcome::Conflict(core::RenameConflict {
                    token,
                    target,
                    can_replace: conflict.can_replace,
                });
                ledger.conflict = Some(Conflict {
                    token,
                    raw: conflict,
                    attempt: number,
                    name: input.value.name.clone(),
                });
                outcome
            }
            api::RenameOutcome::Unchanged(error) => {
                core::RenameOutcome::Unchanged(self.error("rename", error))
            }
            api::RenameOutcome::Renamed {
                name,
                same_entry,
                recovery,
                durability_confirmed,
            } => {
                if name != input.name {
                    return Err(invalid(
                        "Rename destination differs from the immutable input",
                    ));
                }
                core::RenameOutcome::Renamed {
                    name: paths::name(name.encoding, &name.bytes)?,
                    same_entry,
                    recovery: recovery_all(recovery)?,
                    durability_confirmed,
                }
            }
            api::RenameOutcome::RecoveryRequired {
                source_location,
                destination_name,
                recovery,
                error,
            } => {
                if destination_name
                    .as_ref()
                    .is_some_and(|name| name != &input.name)
                {
                    return Err(invalid(
                        "Rename recovery destination differs from its input",
                    ));
                }
                core::RenameOutcome::RecoveryRequired {
                    source_location: match source_location {
                        api::RenameSourceLocation::OriginalName => {
                            core::RenameSourceLocation::OriginalName
                        }
                        api::RenameSourceLocation::Destination => {
                            core::RenameSourceLocation::Destination
                        }
                        api::RenameSourceLocation::Recovery => core::RenameSourceLocation::Recovery,
                        api::RenameSourceLocation::Unknown => core::RenameSourceLocation::Unknown,
                    },
                    destination_name: destination_name
                        .map(|name| paths::name(name.encoding, &name.bytes))
                        .transpose()?,
                    recovery: recovery_all(recovery)?,
                    error: self.error("rename", error),
                }
            }
        };
        ledger.known_attempt = number;
        if let core::RenameOutcome::Renamed { recovery, .. }
        | core::RenameOutcome::RecoveryRequired { recovery, .. } = &outcome
        {
            ledger.recovery = recovery.clone();
        }
        if !matches!(outcome, core::RenameOutcome::Conflict(_)) {
            ledger.conflict = None;
        }
        ledger.state = core::RenameState {
            attempt: number,
            outcome,
        };
        Ok(ledger.state.clone())
    }

    fn close_failure(&self, error: api::Failure) -> core::RenameFinish {
        let error = self.error("close rename", error);
        lock(&self.resources).failure = Some(error.clone());
        core::RenameFinish::Unconfirmed(error)
    }

    fn finish(&self) -> core::RenameFinish {
        {
            let mut ledger = lock(&self.ledger);
            ledger.stopping = true;
            if ledger.handler {
                // Receipt recovery is part of the identifiable active handler.
                // Closing after21 returned but before its GetState would erase
                // the only authoritative result of a possibly completed effect.
                return core::RenameFinish::Executing;
            }
        }
        let _finishing = lock(&self.finishing);
        let _control = lock(&self.control);
        {
            let resources = lock(&self.resources);
            if resources.closed {
                return core::RenameFinish::Closed;
            }
            if let Some(error) = &resources.failure {
                return core::RenameFinish::Unconfirmed(error.clone());
            }
        }
        loop {
            let handle = lock(&self.resources)
                .handles
                .iter()
                .find(|handle| !handle.closed)
                .map(|handle| handle.value);
            let Some(handle) = handle else { break };
            match call(
                self,
                &api::Request::CloseRename { prepared: handle },
                &|| false,
            ) {
                Ok(api::Response::Unit) => {
                    if let Some(value) = lock(&self.resources)
                        .handles
                        .iter_mut()
                        .find(|value| value.value == handle)
                    {
                        value.closed = true;
                    }
                }
                // Active handlers defer before taking the control lane. Busy
                // here has no identifiable remaining call and is not retryable.
                Ok(api::Response::Failed(error)) => return self.close_failure(error),
                Ok(response) => {
                    self.unexpected(&response);
                    return self.close_failure(invalid("Invalid CloseRename acknowledgement"));
                }
                Err(error) => return self.close_failure(error),
            }
        }
        if lock(&self.resources).publication_unknown {
            return self.close_failure(invalid(
                "Unknown native Rename resource publication; ownership retained",
            ));
        }
        let mount = lock(&self.resources).mount.take();
        if let Some(mut mount) = mount {
            let result = catch_unwind(AssertUnwindSafe(|| mount.close()))
                .unwrap_or_else(|_| Err(invalid("Native Rename mount close panicked")));
            // Keep an armed failed lease inside the owned component, never Drop it.
            lock(&self.resources).mount = Some(mount);
            if let Err(error) = result {
                return self.close_failure(error);
            }
        }
        lock(&self.resources).closed = true;
        core::RenameFinish::Closed
    }
}

struct Query {
    active: bool,
    input: u64,
    original: Option<core::FsError>,
}

struct RecoveryBatch {
    attempt: u64,
    records: Vec<core::RenameRecovery>,
}

fn merge_associated_recovery(ledger: &mut Ledger, batch: Option<RecoveryBatch>) {
    if let Some(batch) = batch
        && ledger
            .input
            .as_ref()
            .is_some_and(|input| input.value.number.get() == batch.attempt)
    {
        merge_recovery(&mut ledger.recovery, batch.records);
    }
}

struct Handler<'a>(&'a Owned);
impl Drop for Handler<'_> {
    fn drop(&mut self) {
        lock(&self.0.ledger).handler = false;
    }
}

fn merge_recovery(current: &mut Vec<core::RenameRecovery>, values: Vec<core::RenameRecovery>) {
    let mut bytes = current
        .iter()
        .try_fold(0usize, |bytes, value| {
            bytes.checked_add(paths::encode(&value.path).ok()?.bytes.len())
        })
        .unwrap_or(api::MAX_RENAME_RECOVERY_ADDRESS_BYTES);
    for value in values {
        let Some(total) = paths::encode(&value.path)
            .ok()
            .and_then(|path| bytes.checked_add(path.bytes.len()))
        else {
            continue;
        };
        if current.len() < api::MAX_RENAME_RECOVERY_RECORDS
            && total <= api::MAX_RENAME_RECOVERY_ADDRESS_BYTES
            && !current.contains(&value)
        {
            current.push(value);
            bytes = total;
        }
    }
}

fn same_kind(first: api::Kind, second: api::Kind) -> bool {
    first == second
        || matches!(
            (first, second),
            (api::Kind::Symlink, api::Kind::DirectoryLink)
                | (api::Kind::DirectoryLink, api::Kind::Symlink)
        )
}

fn validate_resolved(node: &api::Node, requested: &api::Address) -> Result<(), api::Failure> {
    if node.address != *requested
        || node.raw_name.len() > api::MAX_RENAME_NAME_BYTES
        || node
            .address
            .bytes
            .len()
            .checked_add(node.raw_name.len())
            .and_then(|bytes| bytes.checked_add(node.display.len()))
            .is_none_or(|bytes| bytes > api::DEFAULT_REPLY_BYTES as usize)
        || node.kind == api::Kind::Other
        || (node.kind != api::Kind::File && node.metadata.size.is_some())
    {
        return Err(invalid(
            "Invalid native Rename Resolve association or bounds",
        ));
    }
    validate_node(node)?;
    observed_name(&api::NativeText {
        encoding: node.name_encoding,
        bytes: node.raw_name.clone(),
    })?;
    entry(node.clone())?;
    Ok(())
}

fn observed_name(name: &api::NativeText) -> Result<OsString, api::Failure> {
    name.validate_rename_observed_basename()
        .map_err(|_| invalid("Invalid observed Rename basename"))?;
    paths::name(name.encoding, &name.bytes)
}

fn object_item(
    resolved: &api::Node,
    object: &api::RenameObject,
    path: &Path,
) -> Result<core::MutationItem, api::Failure> {
    object
        .validate()
        .map_err(|_| invalid("Invalid native Rename object"))?;
    observed_name(&object.name)?;
    let mut node = resolved.clone();
    node.address = paths::encode(path)?;
    node.name_encoding = object.name.encoding;
    node.raw_name = object.name.bytes.clone();
    node.kind = object.kind;
    node.metadata = object.metadata.clone();
    let entry = entry(node)?;
    Ok(core::MutationItem {
        path: entry.path,
        kind: entry.kind,
        metadata: entry.metadata,
    })
}

fn recovery_one(value: &api::RenameRecovery) -> Result<core::RenameRecovery, api::Failure> {
    if value.address.bytes.is_empty()
        || value.address.bytes.len() > api::MAX_RENAME_RECOVERY_ADDRESS_BYTES
        || value.identity.len() > api::MAX_RENAME_IDENTITY_BYTES
    {
        return Err(invalid("Invalid native Rename recovery bounds"));
    }
    Ok(core::RenameRecovery {
        role: match value.role {
            api::RenameRecoveryRole::Source => core::RenameRecoveryRole::Source,
            api::RenameRecoveryRole::DisplacedTarget => core::RenameRecoveryRole::DisplacedTarget,
            api::RenameRecoveryRole::EmptyStage => core::RenameRecoveryRole::EmptyStage,
        },
        path: paths::decode(&value.address)?,
        identity: value.identity.clone(),
    })
}

fn recovery_all(
    values: Vec<api::RenameRecovery>,
) -> Result<Vec<core::RenameRecovery>, api::Failure> {
    values.iter().map(recovery_one).collect()
}

fn recovery_of(owner: &Owned, state: &api::RenameState) -> Option<RecoveryBatch> {
    if owner.prepared.get()?.handle != state.prepared
        || lock(&owner.ledger)
            .input
            .as_ref()
            .is_none_or(|input| input.value.number.get() != state.attempt)
    {
        return None;
    }
    let values = match &state.outcome {
        api::RenameOutcome::Renamed { recovery, .. }
        | api::RenameOutcome::RecoveryRequired { recovery, .. } => recovery,
        _ => return None,
    };
    let mut bytes = 0usize;
    let records = values
        .iter()
        .take(api::MAX_RENAME_RECOVERY_RECORDS)
        .filter_map(|value| {
            let total = bytes.checked_add(value.address.bytes.len())?;
            if total > api::MAX_RENAME_RECOVERY_ADDRESS_BYTES {
                return None;
            }
            let value = recovery_one(value).ok()?;
            bytes = total;
            Some(value)
        })
        .collect();
    Some(RecoveryBatch {
        attempt: state.attempt,
        records,
    })
}

/// Generic wrong-variant cleanup can disclose a prepared handle before a Rename
/// adapter exists. Its complete private owner is retained if close is unconfirmed.
pub(crate) fn close_unexpected(provider: &NativeProvider, handle: u64) {
    if handle == 0 {
        return;
    }
    let resource = Resource::new(provider, Path::new(""));
    lock(&resource.owned().resources).handles.push(Handle {
        value: handle,
        closed: false,
    });
    let _ = core::RenameResource::finish(&resource);
    drop(resource);
}

#[cfg(test)]
#[path = "rename_tests.rs"]
mod tests;
