//! Explicit persistent recovery; Drop only closes descriptors. Never recursively
//! remove a stage or restore over a newly appearing winner.
use crate::{
    rename::{
        ITEM, Identity, Prepared, Revision, changed, limit, local_filesystem, trusted_parent,
    },
    rename_sys as sys,
};
use midnight_filesystem_plugin_api::*;
use std::{
    ffi::CString,
    fs::File,
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

/// Small error carrier: namespace residuals reuse Plan's pre-reserved records.
/// The large public RenameOutcome is assembled only at the adapter boundary.
pub(crate) struct CreateFailure {
    error: Failure,
    source_location: Option<RenameSourceLocation>,
    recovery: Vec<RenameRecovery>,
}
impl CreateFailure {
    fn unchanged(error: Failure) -> Self {
        Self {
            error,
            source_location: None,
            recovery: Vec::new(),
        }
    }
    pub(crate) fn into_outcome(self) -> RenameOutcome {
        match self.source_location {
            None => RenameOutcome::Unchanged(self.error),
            Some(source_location) => RenameOutcome::RecoveryRequired {
                source_location,
                destination_name: None,
                recovery: self.recovery,
                error: self.error,
            },
        }
    }
}
struct Plan {
    #[cfg(test)]
    path: std::path::PathBuf,
    name: CString,
    destination: CString,
    proposed: NativeText,
    receipt: RenameRecovery,
    records: Vec<RenameRecovery>,
}
impl Plan {
    fn new(prepared: &Prepared, proposed: &NativeText) -> Result<Self, Failure> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let sequence = NEXT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| limit())?;
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| limit())?
            .as_nanos();
        let name = CString::new(format!(
            ".midnight-rename-{}-{time:x}-{sequence:x}",
            std::process::id()
        ))
        .map_err(|_| limit())?;
        let path = prepared
            .parent_path
            .join(std::ffi::OsStr::from_bytes(name.as_bytes()));
        let item = path.join("item");
        // Pool the same aggregate address reservation previously used by the
        // source, displaced-target and container records. Only one is emitted.
        // This permits a longer relocated locator without another allocation or
        // a new charge outside the existing prepared Rename resource admission.
        let total = item
            .as_os_str()
            .as_bytes()
            .len()
            .saturating_mul(2)
            .saturating_add(path.as_os_str().as_bytes().len());
        if total > MAX_RENAME_RECOVERY_ADDRESS_BYTES {
            return Err(limit());
        }
        let destination = CString::new(proposed.bytes.as_slice())
            .map_err(|_| Failure::new(ErrorCode::Invalid, "NUL in destination name."))?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(total).map_err(|_| limit())?;
        // Do not hide allocator rounding outside the old aggregate reservation.
        if bytes.capacity() > total {
            return Err(limit());
        }
        let receipt = RenameRecovery {
            role: RenameRecoveryRole::EmptyStage,
            address: Address {
                encoding: AddressEncoding::UnixBytes,
                bytes,
            },
            identity: Vec::with_capacity(20),
        };
        if receipt.identity.capacity() > 3 * 20 {
            return Err(limit());
        }
        let mut records = Vec::new();
        records.try_reserve_exact(1).map_err(|_| limit())?;
        if records.capacity() > 3 {
            return Err(limit());
        }
        Ok(Self {
            #[cfg(test)]
            path,
            name,
            destination,
            proposed: proposed.clone(),
            receipt,
            records,
        })
    }
    fn receipt(
        mut self,
        role: RenameRecoveryRole,
        file: &File,
        expected: Option<Identity>,
    ) -> Result<Vec<RenameRecovery>, Failure> {
        let identity = Identity::of(file)?;
        if expected.is_some_and(|expected| identity != expected) {
            return Err(changed("Retained recovery descriptor changed identity."));
        }
        let visible = sys::observe_locator(file, &mut self.receipt.address.bytes)?;
        if Identity::of(&visible)? != identity || Identity::of(file)? != identity {
            return Err(changed(
                "Current recovery locator no longer names the retained object.",
            ));
        }
        self.receipt.role = role;
        identity.write_bytes(&mut self.receipt.identity);
        self.records.push(self.receipt);
        Ok(self.records)
    }
    fn failed(self, prepared: &Prepared, error: Failure, stage: Option<&File>) -> CreateFailure {
        // mkdir succeeded or its result is ambiguous. Never allocate a Box merely
        // to carry this post-effect error; move the pre-reserved exact records.
        let source_location = Some(if prepared.revalidate().is_ok() {
            RenameSourceLocation::OriginalName
        } else {
            RenameSourceLocation::Unknown
        });
        let observed = match stage {
            Some(file) => self.receipt(RenameRecoveryRole::EmptyStage, file, None),
            None => Err(changed(
                "The created stage descriptor could not be retained.",
            )),
        };
        let (recovery, error) = match observed {
            Ok(recovery) => (recovery, error),
            Err(inspect) => (
                Vec::new(),
                combine(error, "recovery locator", unconfirmed_locator(inspect)),
            ),
        };
        CreateFailure {
            source_location,
            recovery,
            error,
        }
    }
}
fn unconfirmed_locator(mut error: Failure) -> Failure {
    error.message.insert_str(
        0,
        "Current recovery locator could not be confirmed; no recovery address was published: ",
    );
    error
}
fn combine(mut first: Failure, context: &'static str, extra: Failure) -> Failure {
    // Native messages have no unbounded user pathname expansion. Keep both actual errors.
    first.message.push_str("; ");
    first.message.push_str(context);
    first.message.push_str(": ");
    first.message.push_str(&extra.message);
    first
}
pub(crate) struct Stage {
    plan: Plan,
    file: File,
    identity: Identity,
    parent_sync: File,
    stage_sync: File,
    captured: Option<File>,
    item_identity: Option<Identity>,
    item_present: bool,
    exists: bool,
    exchange_target: Option<Identity>,
    publication_committed: bool,
}
impl Stage {
    pub(crate) fn create(prepared: &Prepared, name: &NativeText) -> Result<Self, CreateFailure> {
        // Readable/sync descriptors are acquired only during execution.
        let parent_sync =
            sys::readable_directory(&prepared.parent).map_err(CreateFailure::unchanged)?;
        if Identity::of(&parent_sync).map_err(CreateFailure::unchanged)? != prepared.parent_identity
        {
            return Err(CreateFailure::unchanged(changed(
                "Parent sync descriptor changed identity.",
            )));
        }
        for _ in 0..32 {
            let plan = Plan::new(prepared, name).map_err(CreateFailure::unchanged)?;
            match sys::mkdir(&prepared.parent, &plan.name) {
                Ok(()) => {}
                Err(failure)
                    if failure.os_code == Some(libc::EEXIST)
                        && local_filesystem(&prepared.parent) =>
                {
                    continue;
                }
                Err(failure) if local_filesystem(&prepared.parent) => {
                    return Err(CreateFailure::unchanged(failure));
                }
                Err(failure) => {
                    let file = sys::pin(&prepared.parent, &plan.name).ok();
                    return Err(plan.failed(prepared, combine(failure, "stage creation",
                        Failure::new(ErrorCode::Io, "The failed request may have created this stage on an unqualified filesystem; its creation outcome is unconfirmed.")), file.as_ref()));
                }
            }
            let file = match sys::pin(&prepared.parent, &plan.name) {
                Ok(file) => file,
                Err(failure) => return Err(plan.failed(prepared, failure, None)),
            };
            let identity = match Identity::of(&file) {
                Ok(value) => value,
                Err(failure) => return Err(plan.failed(prepared, failure, Some(&file))),
            };
            let metadata = match file.metadata().map_err(Failure::from_io) {
                Ok(value) => value,
                Err(failure) => return Err(plan.failed(prepared, failure, Some(&file))),
            };
            if !metadata.is_dir()
                || metadata.uid() != sys::effective_uid()
                || metadata.mode() & 0o777 != 0o700
                || identity.device != prepared.parent_identity.device
            {
                return Err(plan.failed(
                    prepared,
                    changed("New Rename stage is not an owner-only same-device directory."),
                    Some(&file),
                ));
            }
            let stage_sync = match sys::readable_directory(&file) {
                Ok(file) => file,
                Err(failure) => return Err(plan.failed(prepared, failure, Some(&file))),
            };
            if Identity::of(&stage_sync).ok() != Some(identity) {
                return Err(plan.failed(
                    prepared,
                    changed("Stage sync descriptor changed identity."),
                    Some(&file),
                ));
            }
            return Ok(Self {
                plan,
                file,
                identity,
                parent_sync,
                stage_sync,
                captured: None,
                item_identity: None,
                item_present: false,
                exists: true,
                exchange_target: None,
                publication_committed: false,
            });
        }
        Err(CreateFailure::unchanged(Failure::new(
            ErrorCode::ResourceLimit,
            "Rename stage collision limit reached.",
        )))
    }
    pub(crate) fn file(&self) -> &File {
        &self.file
    }
    #[cfg(test)]
    pub(crate) fn path(&self) -> &std::path::Path {
        &self.plan.path
    }
    pub(crate) fn item_identity(&self) -> Option<Identity> {
        self.item_identity
    }
    fn container_current(&self, prepared: &Prepared) -> Result<(), Failure> {
        prepared.check_parent()?;
        trusted_parent(&prepared.parent)?;
        let visible = sys::pin(&prepared.parent, &self.plan.name)?;
        if Identity::of(&visible)? != self.identity || Identity::of(&self.file)? != self.identity {
            return Err(changed("Rename stage binding changed."));
        }
        let metadata = self.file.metadata().map_err(Failure::from_io)?;
        if !metadata.is_dir()
            || metadata.uid() != sys::effective_uid()
            || metadata.mode() & 0o777 != 0o700
        {
            return Err(changed("Rename stage protection changed."));
        }
        Ok(())
    }
    pub(crate) fn validate(&self, prepared: &Prepared) -> Result<(), Failure> {
        self.container_current(prepared)?;
        if let Some(captured) = &self.captured {
            let visible = sys::pin(&self.file, ITEM)?;
            if Some(Identity::of(&visible)?) != self.item_identity
                || Some(Identity::of(captured)?) != self.item_identity
            {
                return Err(changed("Captured stage entry changed."));
            }
        }
        Ok(())
    }
    pub(crate) fn pin_item(&mut self) -> Result<(), Failure> {
        self.item_identity = None;
        self.captured = None;
        self.item_present = true;
        let file = match sys::pin(&self.file, ITEM) {
            Ok(file) => file,
            Err(failure) => {
                if failure.code == ErrorCode::NotFound {
                    self.item_present = false;
                }
                self.item_identity = None;
                self.captured = None;
                return Err(failure);
            }
        };
        self.item_identity = Some(Identity::of(&file)?);
        self.captured = Some(file);
        Ok(())
    }
    fn sync_names(&self, prepared: &Prepared) -> Result<(), Failure> {
        let identity_check = (|| {
            if Identity::of(&self.parent_sync)? != prepared.parent_identity
                || Identity::of(&self.stage_sync)? != self.identity
            {
                return Err(changed("A directory sync descriptor changed identity."));
            }
            Ok(())
        })();
        // Both sync calls are attempted even if the first fails. No caller cancellation is consulted.
        let stage = sys::sync(&self.stage_sync);
        let parent = sys::sync(&self.parent_sync);
        identity_check.and(stage).and(parent)
    }
    fn cleanup(&mut self, prepared: &Prepared) -> Result<(), Failure> {
        #[cfg(test)]
        crate::rename_test_hooks::fire(
            crate::rename_test_hooks::Boundary::BeforeCleanup,
            self.path(),
        );
        self.container_current(prepared)?;
        // Empty-only, handle-relative removal. An unexpected child is retained.
        if let Err(failure) = sys::rmdir(&prepared.parent, &self.plan.name) {
            if sys::pin(&prepared.parent, &self.plan.name)
                .is_err_and(|error| error.code == ErrorCode::NotFound)
            {
                self.exists = false;
            }
            return Err(failure);
        }
        self.exists = false; // Record actual namespace fact before the post-removal sync.
        sys::sync(&self.parent_sync)
    }
    fn destination_has_source(&self, prepared: &Prepared) -> bool {
        prepared.check_parent().is_ok()
            && sys::pin(&prepared.parent, &self.plan.destination)
                .ok()
                .and_then(|file| Identity::of(&file).ok())
                == Some(prepared.identity)
    }
    fn recovery_role(
        &self,
        prepared: &Prepared,
        fallback: RenameRecoveryRole,
    ) -> RenameRecoveryRole {
        let Some(consented_target) = self.exchange_target else {
            return RenameRecoveryRole::Source;
        };
        if self.publication_committed || self.item_identity == Some(consented_target) {
            return RenameRecoveryRole::DisplacedTarget;
        }
        if self.item_identity == Some(prepared.identity) {
            return RenameRecoveryRole::Source;
        }
        if self.destination_has_source(prepared) {
            return RenameRecoveryRole::DisplacedTarget;
        }
        fallback
    }
    fn source_location(&self, prepared: &Prepared) -> (RenameSourceLocation, Option<NativeText>) {
        let visible = |name: &std::ffi::CStr| {
            sys::pin(&prepared.parent, name)
                .ok()
                .and_then(|file| Identity::of(&file).ok())
        };
        if prepared.check_parent().is_ok() {
            let original = visible(&prepared.name) == Some(prepared.identity);
            let destination = visible(&self.plan.destination) == Some(prepared.identity);
            match (original, destination) {
                // Object identity cannot distinguish two hard-link names. Keep
                // that ambiguity while retaining the independently observed destination.
                (true, true) => {
                    return (
                        RenameSourceLocation::Unknown,
                        Some(self.plan.proposed.clone()),
                    );
                }
                (true, false) => return (RenameSourceLocation::OriginalName, None),
                (false, true) => {
                    return (
                        RenameSourceLocation::Destination,
                        Some(self.plan.proposed.clone()),
                    );
                }
                (false, false) => {}
            }
        }
        let staged = sys::pin(&self.file, ITEM)
            .ok()
            .and_then(|file| Identity::of(&file).ok());
        if self.container_current(prepared).is_ok() && staged == Some(prepared.identity) {
            return (RenameSourceLocation::Recovery, None);
        }
        (RenameSourceLocation::Unknown, None)
    }
    fn receipts(self, role: RenameRecoveryRole) -> Result<Vec<RenameRecovery>, Failure> {
        if self.item_present {
            let file = self
                .captured
                .as_ref()
                .ok_or_else(|| changed("The current recovery item could not be pinned."))?;
            self.plan.receipt(role, file, self.item_identity)
        } else if self.exists {
            self.plan.receipt(
                RenameRecoveryRole::EmptyStage,
                &self.file,
                Some(self.identity),
            )
        } else {
            Ok(self.plan.records)
        }
    }
    fn required(
        mut self,
        prepared: &Prepared,
        role: RenameRecoveryRole,
        error: Failure,
    ) -> RenameOutcome {
        // Refresh after any failed validation. A previous pin is not evidence
        // that its inode still occupies the current recovery locator.
        let _ = self.pin_item();
        let role = self.recovery_role(prepared, role);
        let (source_location, destination_name) = self.source_location(prepared);
        let (recovery, error) = match self.receipts(role) {
            Ok(recovery) => (recovery, error),
            Err(inspect) => (
                Vec::new(),
                combine(error, "recovery locator", unconfirmed_locator(inspect)),
            ),
        };
        RenameOutcome::RecoveryRequired {
            source_location,
            destination_name,
            recovery,
            error,
        }
    }
    pub(crate) fn without_capture(mut self, prepared: &Prepared, error: Failure) -> RenameOutcome {
        // A failed namespace syscall can be ambiguous on remote storage. Observe
        // the retained stage before claiming that capture did not happen.
        match self.pin_item() {
            Ok(()) => return self.restore(prepared, error),
            Err(inspect) if inspect.code == ErrorCode::NotFound => {
                if !local_filesystem(&prepared.parent) {
                    return self.required(
                        prepared,
                        RenameRecoveryRole::Source,
                        combine(
                            error,
                            "capture outcome",
                            Failure::new(
                                ErrorCode::Io,
                                "The failed native request is ambiguous on this filesystem; the observed stage was retained.",
                            ),
                        ),
                    );
                }
            }
            Err(inspect) => {
                return self.required(
                    prepared,
                    RenameRecoveryRole::Source,
                    combine(error, "failed-capture inspection", inspect),
                );
            }
        }
        if let Err(sync) = self.sync_names(prepared) {
            return self.required(
                prepared,
                RenameRecoveryRole::Source,
                combine(error, "directory durability", sync),
            );
        }
        match self.cleanup(prepared) {
            Ok(()) => RenameOutcome::Unchanged(error),
            Err(cleanup) => self.required(
                prepared,
                RenameRecoveryRole::Source,
                combine(error, "stage cleanup", cleanup),
            ),
        }
    }
    pub(crate) fn restore(mut self, prepared: &Prepared, error: Failure) -> RenameOutcome {
        // A failed publication may actually have committed. Observe the actual
        // slot before deciding what remains; never report a nonexistent staged source.
        match sys::pin(&self.file, ITEM) {
            Ok(visible) => match Identity::of(&visible) {
                Ok(identity) if Some(identity) == self.item_identity && self.captured.is_some() => {
                }
                Ok(identity) => {
                    self.item_identity = Some(identity);
                    self.item_present = true;
                    self.captured = Some(visible);
                    return self.required(
                        prepared,
                        RenameRecoveryRole::Source,
                        combine(
                            error,
                            "restoration validation",
                            changed("Staged object changed; no restoration was attempted."),
                        ),
                    );
                }
                Err(inspect) => {
                    self.item_identity = None;
                    self.item_present = true;
                    self.captured = None;
                    return self.required(
                        prepared,
                        RenameRecoveryRole::Source,
                        combine(error, "restoration identity", inspect),
                    );
                }
            },
            Err(inspect) => {
                self.item_identity = None;
                self.item_present = inspect.code != ErrorCode::NotFound;
                self.captured = None;
                return self.required(
                    prepared,
                    RenameRecoveryRole::Source,
                    combine(error, "restoration slot inspection", inspect),
                );
            }
        }
        // Revalidate the actual captured object, even when it was a raced substitute.
        // Never replace either an original-name winner or a changed staged entry.
        #[cfg(test)]
        crate::rename_test_hooks::fire(
            crate::rename_test_hooks::Boundary::BeforeRestoreValidation,
            self.path(),
        );
        if let Err(validation) = self.validate(prepared) {
            return self.required(
                prepared,
                RenameRecoveryRole::Source,
                combine(error, "restoration validation", validation),
            );
        }
        if self.captured.is_none() {
            return self.required(prepared, RenameRecoveryRole::Source, error);
        }
        if let Err(restore) = sys::rename(&self.file, ITEM, &prepared.parent, &prepared.name, false)
        {
            let inspection = self.pin_item();
            let error = combine(error, "no-replace restoration returned an error", restore);
            let error = match inspection {
                Ok(()) => error,
                Err(inspect) if inspect.code == ErrorCode::NotFound => error,
                Err(inspect) => combine(error, "post-restoration observation", inspect),
            };
            return self.required(prepared, RenameRecoveryRole::Source, error);
        }
        self.captured = None;
        self.item_identity = None;
        self.item_present = false;
        if let Err(sync) = self.sync_names(prepared) {
            return self.required(
                prepared,
                RenameRecoveryRole::Source,
                combine(error, "restored-name durability", sync),
            );
        }
        match self.cleanup(prepared) {
            Ok(()) => RenameOutcome::Unchanged(error),
            Err(cleanup) => self.required(
                prepared,
                RenameRecoveryRole::Source,
                combine(error, "restored-stage cleanup", cleanup),
            ),
        }
    }
    pub(crate) fn mark_publication(&mut self, consented_target: Option<Identity>) {
        self.exchange_target = consented_target;
    }
    pub(crate) fn committed(
        mut self,
        prepared: &Prepared,
        destination: &CString,
        name: &NativeText,
        replaced: Option<(Identity, Revision)>,
    ) -> RenameOutcome {
        // Native commit has happened. All remaining work ignores user cancellation.
        self.publication_committed = true;
        self.captured = None;
        self.item_identity = None;
        self.item_present = replaced.is_some();
        let item_check = if replaced.is_some() {
            self.pin_item()
        } else {
            Ok(())
        };
        let checked = (|| {
            self.container_current(prepared)?;
            let published = sys::pin(&prepared.parent, destination)?;
            if Identity::of(&published)? != prepared.identity
                || Identity::of(&prepared.source)? != prepared.identity
            {
                return Err(changed(
                    "Native publication completed but source destination identity changed.",
                ));
            }
            if let Some((expected, revision)) = replaced {
                item_check?;
                let displaced = self
                    .captured
                    .as_ref()
                    .ok_or_else(|| changed("Displaced target could not be pinned."))?;
                if self.item_identity != Some(expected)
                    || !revision.after_rename_matches(Revision::of(displaced)?)
                {
                    return Err(changed(
                        "A concurrently changed destination was retained in recovery.",
                    ));
                }
            }
            Ok(())
        })();
        let synced = self.sync_names(prepared);
        if let Err(error) = checked {
            let error = match synced {
                Ok(()) => error,
                Err(sync) => combine(error, "directory durability", sync),
            };
            return self.required(prepared, RenameRecoveryRole::DisplacedTarget, error);
        }
        if let Err(error) = synced {
            return self.required(prepared, RenameRecoveryRole::DisplacedTarget, error);
        }
        if replaced.is_none() {
            if let Err(error) = self.cleanup(prepared) {
                return self.required(prepared, RenameRecoveryRole::DisplacedTarget, error);
            }
            return RenameOutcome::Renamed {
                name: name.clone(),
                same_entry: false,
                recovery: vec![],
                durability_confirmed: true,
            };
        }
        let (source_location, destination_name) = self.source_location(prepared);
        match self.receipts(RenameRecoveryRole::DisplacedTarget) {
            Ok(recovery) => RenameOutcome::Renamed {
                name: name.clone(),
                same_entry: false,
                recovery,
                durability_confirmed: true,
            },
            Err(error) => RenameOutcome::RecoveryRequired {
                source_location,
                destination_name,
                recovery: Vec::new(),
                error: unconfirmed_locator(error),
            },
        }
    }
}
