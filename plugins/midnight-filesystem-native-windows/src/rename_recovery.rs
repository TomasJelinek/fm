//! Qualified handle-relative replacement with explicit, retained recovery facts.

#[cfg(test)]
use crate::rename::{address, text};
use crate::{
    rename::{Point, Prepared, Target, changed, checked, io_failure, point},
    rename_native as native,
};
use midnight_filesystem_plugin_api::*;
use midnight_filesystem_plugin_sdk::{CallContext, native::RENAME_RESOURCE_BYTES};
use std::{
    ffi::{OsStr, OsString},
    fs::File,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
};

// Reserve both possible residual locators and one reusable native path buffer.
// NTFS paths fit within these bounds; the API permits a larger aggregate because
// a failed operation can retain more than one independent residual.
const LOCATOR_BYTES: usize = MAX_RENAME_RECOVERY_ADDRESS_BYTES / 2;
const PATH_UNITS: usize = LOCATOR_BYTES / 2 + 1;
const PRIVATE_CREATE_BYTES: usize = 64 * 1024;

fn resource_limit() -> Failure {
    Failure::new(
        ErrorCode::ResourceLimit,
        "Native Rename recovery buffers exceed the reserved operation budget.",
    )
}

struct Budget(usize);
impl Budget {
    fn charge(&mut self, bytes: usize) -> Result<(), Failure> {
        self.0 = self.0.checked_add(bytes).ok_or_else(resource_limit)?;
        if self.0 > RENAME_RESOURCE_BYTES {
            return Err(resource_limit());
        }
        Ok(())
    }
}

fn bytes(capacity: usize, budget: &mut Budget) -> Result<Vec<u8>, Failure> {
    budget.charge(capacity)?;
    let mut value = Vec::new();
    value
        .try_reserve_exact(capacity)
        .map_err(|_| resource_limit())?;
    budget.charge(value.capacity().saturating_sub(capacity))?;
    Ok(value)
}

fn joined(parent: &Path, name: &OsStr, budget: &mut Budget) -> Result<PathBuf, Failure> {
    let capacity = parent
        .as_os_str()
        .len()
        .checked_add(name.len())
        .and_then(|length| length.checked_add(1))
        .ok_or_else(resource_limit)?;
    budget.charge(capacity)?;
    let mut path = PathBuf::new();
    path.try_reserve_exact(capacity)
        .map_err(|_| resource_limit())?;
    path.push(parent);
    path.push(name);
    budget.charge(path.capacity().saturating_sub(capacity))?;
    Ok(path)
}

fn encode_path(path: &Path, output: &mut Vec<u8>) -> Result<(), Failure> {
    let length = path
        .as_os_str()
        .encode_wide()
        .count()
        .checked_mul(2)
        .ok_or_else(resource_limit)?;
    if length == 0 || length > LOCATOR_BYTES || length > output.capacity() {
        return Err(resource_limit());
    }
    output.clear();
    output.extend(path.as_os_str().encode_wide().flat_map(u16::to_le_bytes));
    Ok(())
}

fn record(
    role: RenameRecoveryRole,
    path: &Path,
    budget: &mut Budget,
) -> Result<RenameRecovery, Failure> {
    let mut address_bytes = bytes(LOCATOR_BYTES, budget)?;
    encode_path(path, &mut address_bytes)?;
    Ok(RenameRecovery {
        role,
        address: Address {
            encoding: AddressEncoding::WindowsUtf16Le,
            bytes: address_bytes,
        },
        identity: bytes(MAX_RENAME_IDENTITY_BYTES, budget)?,
    })
}

fn append_diagnostic(output: &mut String, detail: &str) {
    let remaining = MAX_DIAGNOSTIC.saturating_sub(output.len());
    let mut length = detail.len().min(remaining);
    while !detail.is_char_boundary(length) {
        length -= 1;
    }
    output.push_str(&detail[..length]);
}

struct Plan {
    stage_name: OsString,
    stage_path: PathBuf,
    destination_path: PathBuf,
    restore_path: PathBuf,
    destination: NativeText,
    stage_record: RenameRecovery,
    target_record: RenameRecovery,
    records: Vec<RenameRecovery>,
    path_units: Vec<u16>,
    capture: native::RenameBuffer,
    publish: native::RenameBuffer,
    restore: native::RenameBuffer,
    failure: Failure,
    stage_observed: bool,
}

impl Plan {
    fn new(prepared: &Prepared, target: &Target, name: &OsStr) -> Result<Self, Failure> {
        // The SDK also owns request/receipt storage, and private directory create
        // temporarily owns bounded token/ACL buffers. Account for those while
        // this module's scratch/receipt allocations are still live.
        let mut budget = Budget(prepared.retained_capacity());
        budget.charge(MAX_RENAME_RECEIPT_BYTES)?;
        budget.charge(PRIVATE_CREATE_BYTES)?;
        // The retained target owns distinct requested and observed basenames.
        // Include both capacities before reserving any stage/publication buffers.
        budget.charge(target.retained_capacity())?;
        // The current request, policy and commit snapshot also remain live.
        budget.charge(
            name.len()
                .saturating_mul(2)
                .saturating_add(MAX_REVISION)
                .saturating_add(size_of::<native::Snapshot>()),
        )?;
        budget.charge(MAX_DIAGNOSTIC * 3)?;
        let parent_path = native::final_path(&prepared.parent).map_err(io_failure)?;
        budget.charge(parent_path.capacity())?;
        let stage_name = native::random_stage_name().map_err(io_failure)?;
        budget.charge(stage_name.capacity())?;
        let stage_path = joined(&parent_path, &stage_name, &mut budget)?;
        let original_path = joined(&stage_path, OsStr::new("original"), &mut budget)?;
        let destination_path = joined(&parent_path, name, &mut budget)?;
        let restore_path = joined(&parent_path, &target.name, &mut budget)?;
        let stage_record = record(RenameRecoveryRole::EmptyStage, &stage_path, &mut budget)?;
        let mut target_record = record(
            RenameRecoveryRole::DisplacedTarget,
            &original_path,
            &mut budget,
        )?;
        target
            .snapshot
            .identity
            .append_bytes(&mut target_record.identity);
        let name_length = name
            .encode_wide()
            .count()
            .checked_mul(2)
            .ok_or_else(resource_limit)?;
        if name_length > MAX_RENAME_NAME_BYTES {
            return Err(resource_limit());
        }
        let mut destination_bytes = bytes(name_length, &mut budget)?;
        destination_bytes.extend(name.encode_wide().flat_map(u16::to_le_bytes));
        let destination = NativeText {
            encoding: NameEncoding::WindowsUtf16Le,
            bytes: destination_bytes,
        };
        destination
            .validate_rename_basename()
            .map_err(|_| resource_limit())?;
        let mut path_units = Vec::new();
        budget.charge(PATH_UNITS * size_of::<u16>())?;
        path_units
            .try_reserve_exact(PATH_UNITS)
            .map_err(|_| resource_limit())?;
        budget.charge(
            path_units
                .capacity()
                .saturating_sub(PATH_UNITS)
                .saturating_mul(size_of::<u16>()),
        )?;
        path_units.resize(PATH_UNITS, 0);
        let mut records = Vec::new();
        let record_bytes = MAX_RENAME_RECOVERY_RECORDS * size_of::<RenameRecovery>();
        budget.charge(record_bytes)?;
        records
            .try_reserve_exact(MAX_RENAME_RECOVERY_RECORDS)
            .map_err(|_| resource_limit())?;
        budget.charge(
            records
                .capacity()
                .saturating_sub(MAX_RENAME_RECOVERY_RECORDS)
                .saturating_mul(size_of::<RenameRecovery>()),
        )?;
        let capture = native::RenameBuffer::new(OsStr::new("original")).map_err(io_failure)?;
        budget.charge(capture.retained_capacity())?;
        let publish = native::RenameBuffer::new(name).map_err(io_failure)?;
        budget.charge(publish.retained_capacity())?;
        let restore = native::RenameBuffer::new(&target.name).map_err(io_failure)?;
        budget.charge(restore.retained_capacity())?;
        let mut message = String::new();
        budget.charge(MAX_DIAGNOSTIC)?;
        message
            .try_reserve_exact(MAX_DIAGNOSTIC)
            .map_err(|_| resource_limit())?;
        budget.charge(message.capacity().saturating_sub(MAX_DIAGNOSTIC))?;
        budget.charge(size_of::<Self>())?;
        if stage_record
            .address
            .bytes
            .len()
            .checked_add(target_record.address.bytes.len())
            .is_none_or(|length| length > MAX_RENAME_RECOVERY_ADDRESS_BYTES)
        {
            return Err(resource_limit());
        }
        Ok(Self {
            stage_name,
            stage_path,
            destination_path,
            restore_path,
            destination,
            stage_record,
            target_record,
            records,
            path_units,
            capture,
            publish,
            restore,
            failure: Failure {
                code: ErrorCode::Io,
                os_code: None,
                message,
            },
            stage_observed: false,
        })
    }

    fn fail(&mut self, error: Failure) {
        self.failure.code = error.code;
        self.failure.os_code = error.os_code;
        self.failure.message.clear();
        append_diagnostic(&mut self.failure.message, &error.message);
    }

    fn supplement(&mut self, context: &'static str, error: &Failure) {
        append_diagnostic(&mut self.failure.message, context);
        append_diagnostic(&mut self.failure.message, &error.message);
    }

    fn observe_stage(&mut self, stage: &File) -> Result<(), Failure> {
        observe(stage, &mut self.path_units, &mut self.stage_record.address)?;
        self.stage_observed = true;
        // The target is captured into the exact held stage under this fixed raw
        // name. Reserve/check this second locator before moving that target.
        const SUFFIX: &[u8] = &[
            92, 0, 111, 0, 114, 0, 105, 0, 103, 0, 105, 0, 110, 0, 97, 0, 108, 0,
        ];
        let length = self
            .stage_record
            .address
            .bytes
            .len()
            .checked_add(SUFFIX.len())
            .ok_or_else(resource_limit)?;
        if length > LOCATOR_BYTES || length > self.target_record.address.bytes.capacity() {
            return Err(resource_limit());
        }
        self.target_record.address.bytes.clear();
        self.target_record
            .address
            .bytes
            .extend_from_slice(&self.stage_record.address.bytes);
        self.target_record.address.bytes.extend_from_slice(SUFFIX);
        Ok(())
    }

    fn observe_target(&mut self, target: &File) -> Result<(), Failure> {
        observe(
            target,
            &mut self.path_units,
            &mut self.target_record.address,
        )
    }

    fn stage_residual(mut self) -> RenameOutcome {
        if !self.stage_observed {
            append_diagnostic(
                &mut self.failure.message,
                "; recovery stage locator could not be confirmed after creation; \
                 the record retains the last handle-derived parent/name observation",
            );
        }
        self.records.push(self.stage_record);
        RenameOutcome::RecoveryRequired {
            source_location: RenameSourceLocation::OriginalName,
            destination_name: None,
            recovery: self.records,
            error: self.failure,
        }
    }

    fn displaced_residual(mut self, published: bool) -> RenameOutcome {
        self.records.push(self.target_record);
        RenameOutcome::RecoveryRequired {
            source_location: if published {
                RenameSourceLocation::Destination
            } else {
                RenameSourceLocation::OriginalName
            },
            destination_name: published.then_some(self.destination),
            recovery: self.records,
            error: self.failure,
        }
    }

    fn renamed(mut self) -> RenameOutcome {
        self.records.push(self.target_record);
        RenameOutcome::Renamed {
            name: self.destination,
            same_entry: false,
            recovery: self.records,
            // This namespace operation does not claim a successful volume sync.
            durability_confirmed: false,
        }
    }
}

fn observe(file: &File, scratch: &mut [u16], output: &mut Address) -> Result<(), Failure> {
    let count = native::final_path_units(file, scratch).map_err(io_failure)?;
    let length = count.checked_mul(2).ok_or_else(resource_limit)?;
    if count == 0 || length > LOCATOR_BYTES || length > output.bytes.capacity() {
        return Err(resource_limit());
    }
    output.bytes.clear();
    output
        .bytes
        .extend(scratch[..count].iter().copied().flat_map(u16::to_le_bytes));
    Ok(())
}

fn cleanup_empty(stage: File, mut plan: Plan) -> RenameOutcome {
    let cleanup = point(Point::BeforeCleanup, &plan.stage_path)
        .and_then(|()| native::remove_empty_directory(&stage).map_err(io_failure));
    if let Err(error) = cleanup {
        plan.supplement("; owned stage cleanup failed: ", &error);
        // This operation never enumerates or deletes stage children. A nonempty
        // directory is an explicit retained residual, regardless of its origin.
        if let Err(error) = plan.observe_stage(&stage) {
            plan.supplement("; stage locator confirmation failed: ", &error);
        }
        if plan.stage_record.identity.is_empty() {
            match native::identity(&stage) {
                Ok(identity) => identity.append_bytes(&mut plan.stage_record.identity),
                Err(error) => plan.supplement(
                    "; stage identity could not be confirmed: ",
                    &io_failure(error),
                ),
            }
        }
        // Closing a pin performs no recursive cleanup or restoration.
        drop(stage);
        return plan.stage_residual();
    }
    // Explicit exact-handle disposition succeeded; close that owned handle
    // before reporting that no provider-created namespace residual remains.
    drop(stage);
    RenameOutcome::Unchanged(plan.failure)
}

fn restore_target(
    prepared: &Prepared,
    target: &File,
    stage: File,
    mut plan: Plan,
) -> RenameOutcome {
    // Recovery cannot be canceled. Even a failed source/parent revalidation must
    // not prevent an attempt to restore into the exact retained parent object.
    let restored = point(Point::BeforeRestore, &plan.restore_path).and_then(|()| {
        plan.restore
            .apply(target, Some(&prepared.parent))
            .map_err(io_failure)
    });
    if let Err(error) = restored {
        plan.supplement("; target restoration refused without replacement: ", &error);
        if let Err(error) = plan.observe_stage(&stage) {
            plan.supplement("; stage locator confirmation failed: ", &error);
        }
        if let Err(error) = plan.observe_target(target) {
            plan.supplement(
                "; displaced-target locator could not be reconfirmed; \
                 retaining its last confirmed handle-derived observation: ",
                &error,
            );
        }
        drop(stage);
        return plan.displaced_residual(false);
    }
    cleanup_empty(stage, plan)
}

fn after_publication(
    prepared: &Prepared,
    target: &File,
    stage: File,
    mut plan: Plan,
) -> RenameOutcome {
    // Publication has succeeded. Never check cancellation or try restoration
    // here: either would risk replacing or misreporting the actual source.
    let mut failed = false;
    if let Err(error) = point(Point::AfterPublish, &plan.destination_path) {
        plan.fail(error);
        failed = true;
    }
    if let Err(error) = prepared.validate_parent() {
        if failed {
            plan.supplement("; parent binding changed after publication: ", &error);
        } else {
            plan.fail(error);
            failed = true;
        }
    }
    if let Err(error) = plan.observe_stage(&stage) {
        if failed {
            plan.supplement("; stage locator confirmation failed: ", &error);
        } else {
            plan.fail(error);
            failed = true;
        }
    }
    if let Err(error) = plan.observe_target(target) {
        if failed {
            plan.supplement(
                "; displaced-target locator could not be reconfirmed; \
                 retaining its last confirmed handle-derived observation: ",
                &error,
            );
        } else {
            plan.fail(error);
            append_diagnostic(
                &mut plan.failure.message,
                "; displaced-target locator could not be reconfirmed; \
                 the record retains its last confirmed handle-derived observation",
            );
            failed = true;
        }
    }
    drop(stage);
    if failed {
        plan.displaced_residual(true)
    } else {
        plan.renamed()
    }
}

/// The policy caller has already qualified distinct ordinary files, the target's
/// consent revision and local fixed NTFS. Both incoming guards stay alive through
/// capture, publication, restoration and the final receipt construction.
pub(super) fn replace(
    prepared: &Prepared,
    source: File,
    target: File,
    expected: &Target,
    name: &OsStr,
    context: &CallContext<'_>,
) -> RenameOutcome {
    let target_identity = expected.snapshot.identity;
    let mut plan = match Plan::new(prepared, expected, name) {
        Ok(plan) => plan,
        Err(error) => return RenameOutcome::Unchanged(error),
    };
    if let Err(error) = checked(context)
        .and_then(|()| prepared.revalidate(false).map(|_| ()))
        .and_then(|()| {
            let actual = native::identity(&target).map_err(io_failure)?;
            if actual == target_identity {
                Ok(())
            } else {
                Err(changed())
            }
        })
    {
        return RenameOutcome::Unchanged(error);
    }
    let stage = match native::create_private_directory(&prepared.parent, &plan.stage_name) {
        Ok(stage) => stage,
        Err(error) => return RenameOutcome::Unchanged(io_failure(error)),
    };
    // From this point onward every failure explicitly retires the owned empty
    // stage or discloses its exact observed locator. No Drop path mutates files.
    match native::identity(&stage) {
        Ok(identity) => identity.append_bytes(&mut plan.stage_record.identity),
        Err(error) => {
            plan.fail(io_failure(error));
            return cleanup_empty(stage, plan);
        }
    }
    if let Err(error) = plan.observe_stage(&stage) {
        plan.fail(error);
        return cleanup_empty(stage, plan);
    }
    let capture = point(Point::AfterStage, &plan.stage_path)
        .and_then(|()| checked(context))
        .and_then(|()| prepared.revalidate(false).map(|_| ()))
        .and_then(|()| {
            plan.capture
                .apply(&target, Some(&stage))
                .map_err(io_failure)
        });
    if let Err(error) = capture {
        plan.fail(error);
        return cleanup_empty(stage, plan);
    }

    let publish = plan
        .observe_stage(&stage)
        .and_then(|()| plan.observe_target(&target))
        .and_then(|()| point(Point::AfterCapture, &plan.destination_path))
        .and_then(|()| checked(context))
        .and_then(|()| prepared.revalidate(false).map(|_| ()))
        .and_then(|()| point(Point::BeforePublish, &plan.destination_path))
        .and_then(|()| checked(context))
        .and_then(|()| prepared.revalidate(false).map(|_| ()))
        .and_then(|()| {
            plan.publish
                .apply(&source, Some(&prepared.parent))
                .map_err(io_failure)
        });
    if let Err(error) = publish {
        plan.fail(error);
        return restore_target(prepared, &target, stage, plan);
    }
    after_publication(prepared, &target, stage, plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rename::tests::set_hook;
    use crate::test_support::{Client, Temp};
    use std::{
        ffi::OsString,
        fs,
        os::windows::ffi::OsStringExt,
        path::{Path, PathBuf},
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };

    struct Fixture {
        // Instance disposal closes every remaining pin before Temp removes files.
        client: Client,
        mount: u64,
        prepared: PreparedRename,
        consent: RenameConflict,
        source: PathBuf,
        target: PathBuf,
        source_identity: native::Identity,
        target_identity: native::Identity,
        temp: Temp,
    }

    fn identity_at(path: &Path) -> native::Identity {
        let parent = native::open_parent(path.parent().unwrap()).unwrap();
        let file = native::open_child(&parent, path.file_name().unwrap(), native::Access::Observe)
            .unwrap();
        native::identity(&file).unwrap()
    }

    fn state(response: Response) -> RenameState {
        let Response::RenameState(state) = response else {
            panic!("Expected a retained Rename receipt: {response:?}");
        };
        state
    }

    fn decode(value: &Address) -> PathBuf {
        assert_eq!(value.encoding, AddressEncoding::WindowsUtf16Le);
        let (pairs, remainder) = value.bytes.as_chunks::<2>();
        assert!(remainder.is_empty());
        let units: Vec<_> = pairs.iter().copied().map(u16::from_le_bytes).collect();
        assert!(!units.contains(&0));
        PathBuf::from(OsString::from_wide(&units))
    }

    fn injected() -> Failure {
        Failure::new(ErrorCode::Io, "Controlled private Rename failure.")
    }

    impl Fixture {
        fn new() -> Self {
            let temp = Temp::new();
            let source = temp.path.join("source.bin");
            let target = temp.path.join("target.bin");
            fs::write(&source, b"selected source").unwrap();
            fs::write(&target, b"displaced target").unwrap();
            let parent = native::open_parent(&temp.path).unwrap();
            assert!(
                native::local_fixed_ntfs(&parent),
                "Fixture limitation: these native recovery tests require local fixed NTFS"
            );
            drop(parent);
            let source_identity = identity_at(&source);
            let target_identity = identity_at(&target);
            assert_ne!(source_identity, target_identity);
            let client = Client::new();
            let (mount, node) = client.resolve(&source);
            let response = client.call(Request::PrepareRename {
                mount,
                node: node.id,
            });
            let Response::RenamePrepared(prepared) = response else {
                panic!("Native prepared Rename required: {response:?}");
            };
            assert!(prepared.recoverable_replace);
            let first = state(client.call(Request::Rename {
                prepared: prepared.handle,
                attempt: 1,
                name: text(OsStr::new("target.bin")),
                policy: RenamePolicy::NoReplace,
            }));
            let RenameOutcome::Conflict(consent) = first.outcome else {
                panic!("Expected file/file conflict: {first:?}");
            };
            assert!(consent.can_replace);
            assert_eq!(prepared.source.identity, source_identity.bytes());
            assert_eq!(consent.target.identity, target_identity.bytes());
            Self {
                client,
                mount,
                prepared,
                consent,
                source,
                target,
                source_identity,
                target_identity,
                temp,
            }
        }

        fn request(&self) -> Request {
            Request::Rename {
                prepared: self.prepared.handle,
                attempt: 2,
                name: text(OsStr::new("target.bin")),
                policy: RenamePolicy::RecoverableCurrentEntry {
                    conflict: self.consent.token,
                },
            }
        }

        fn replace(&self, cancellation: &AtomicBool) -> RenameState {
            state(
                native::test_call(
                    &self.client.api,
                    self.client.instance,
                    self.request(),
                    cancellation,
                )
                .unwrap(),
            )
        }

        fn query(&self) -> RenameState {
            state(self.client.cancelled(
                Request::GetRenameState {
                    prepared: self.prepared.handle,
                },
                1,
            ))
        }

        fn assert_originals(&self) {
            assert_eq!(identity_at(&self.source), self.source_identity);
            assert_eq!(identity_at(&self.target), self.target_identity);
            assert_eq!(fs::read(&self.source).unwrap(), b"selected source");
            assert_eq!(fs::read(&self.target).unwrap(), b"displaced target");
        }

        fn assert_no_stage(&self) {
            let mut names: Vec<_> = fs::read_dir(&self.temp.path)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect();
            names.sort();
            assert_eq!(
                names,
                vec![OsString::from("source.bin"), OsString::from("target.bin")]
            );
        }

        fn displaced(&self, recovery: &[RenameRecovery]) -> PathBuf {
            assert_eq!(recovery.len(), 1);
            let receipt = &recovery[0];
            assert_eq!(receipt.role, RenameRecoveryRole::DisplacedTarget);
            assert_eq!(receipt.identity, self.target_identity.bytes());
            let path = decode(&receipt.address);
            assert_eq!(address(&path), receipt.address);
            assert!(path.is_absolute());
            assert!(
                path.canonicalize()
                    .unwrap()
                    .starts_with(self.temp.path.canonicalize().unwrap())
            );
            assert_eq!(identity_at(&path), self.target_identity);
            assert_eq!(fs::read(&path).unwrap(), b"displaced target");
            path
        }

        fn close(self) {
            assert_eq!(
                self.client.cancelled(
                    Request::CloseRename {
                        prepared: self.prepared.handle,
                    },
                    1,
                ),
                Response::Unit
            );
            assert_eq!(
                self.client.call(Request::CloseMount { mount: self.mount }),
                Response::Unit
            );
        }
    }

    #[test]
    fn cancellation_after_capture_restores_without_consulting_cancelled_token() {
        let fixture = Fixture::new();
        let cancellation = Arc::new(AtomicBool::new(false));
        let hook_cancel = cancellation.clone();
        let restored = Arc::new(AtomicBool::new(false));
        let hook_restored = restored.clone();
        let _hook = set_hook(move |point, _path| {
            if point == Point::AfterCapture {
                hook_cancel.store(true, Ordering::Release);
            }
            if point == Point::BeforeRestore {
                assert!(hook_cancel.load(Ordering::Acquire));
                hook_restored.store(true, Ordering::Release);
            }
            Ok(())
        });
        let result = fixture.replace(&cancellation);
        assert!(
            matches!(
                &result.outcome,
                RenameOutcome::Unchanged(Failure {
                    code: ErrorCode::Cancelled,
                    ..
                })
            ),
            "{result:?}"
        );
        assert!(restored.load(Ordering::Acquire));
        fixture.assert_originals();
        fixture.assert_no_stage();
        assert_eq!(fixture.query(), result);
        fixture.close();
    }

    #[test]
    fn cancellation_after_publication_preserves_actual_renamed_receipt_through_real_abi() {
        let fixture = Fixture::new();
        let cancellation = Arc::new(AtomicBool::new(false));
        let hook_cancel = cancellation.clone();
        let _hook = set_hook(move |point, _path| {
            if point == Point::AfterPublish {
                hook_cancel.store(true, Ordering::Release);
            }
            Ok(())
        });
        let result = fixture.replace(&cancellation);
        assert!(cancellation.load(Ordering::Acquire));
        let RenameOutcome::Renamed {
            name,
            same_entry,
            recovery,
            durability_confirmed,
        } = &result.outcome
        else {
            panic!("A late cancellation cannot erase committed facts: {result:?}");
        };
        assert_eq!(name, &text(OsStr::new("target.bin")));
        assert!(!same_entry);
        assert!(!durability_confirmed);
        assert!(!fixture.source.exists());
        assert_eq!(identity_at(&fixture.target), fixture.source_identity);
        assert_eq!(fs::read(&fixture.target).unwrap(), b"selected source");
        fixture.displaced(recovery);
        assert_eq!(fixture.query(), result);
        assert_eq!(state(fixture.client.call(fixture.request())), result);
        fixture.close();
    }

    #[test]
    fn failed_restoration_preserves_new_winner_and_exact_displaced_target() {
        let fixture = Fixture::new();
        let _hook = set_hook(move |point, path| {
            if point == Point::AfterCapture {
                return Err(injected());
            }
            if point == Point::BeforeRestore {
                fs::write(path, b"new independent winner").unwrap();
            }
            Ok(())
        });
        let result = fixture.replace(&AtomicBool::new(false));
        let RenameOutcome::RecoveryRequired {
            source_location,
            destination_name,
            recovery,
            ..
        } = &result.outcome
        else {
            panic!("Restoration refusal must retain exact recovery facts: {result:?}");
        };
        assert_eq!(*source_location, RenameSourceLocation::OriginalName);
        assert_eq!(*destination_name, None);
        assert_eq!(identity_at(&fixture.source), fixture.source_identity);
        assert_eq!(fs::read(&fixture.source).unwrap(), b"selected source");
        assert_eq!(
            fs::read(&fixture.target).unwrap(),
            b"new independent winner"
        );
        let winner = identity_at(&fixture.target);
        assert_ne!(winner, fixture.source_identity);
        assert_ne!(winner, fixture.target_identity);
        fixture.displaced(recovery);
        assert_eq!(fixture.query(), result);
        fixture.close();
    }

    #[test]
    fn cleanup_failure_after_stage_creation_or_restoration_reports_owned_stage() {
        for fail_after_capture in [false, true] {
            let fixture = Fixture::new();
            let _hook = set_hook(move |point, _path| {
                if point
                    == if fail_after_capture {
                        Point::AfterCapture
                    } else {
                        Point::AfterStage
                    }
                    || point == Point::BeforeCleanup
                {
                    return Err(injected());
                }
                Ok(())
            });
            let result = fixture.replace(&AtomicBool::new(false));
            let RenameOutcome::RecoveryRequired {
                source_location,
                destination_name,
                recovery,
                ..
            } = &result.outcome
            else {
                panic!("Owned stage cleanup failure cannot be Unchanged: {result:?}");
            };
            assert_eq!(*source_location, RenameSourceLocation::OriginalName);
            assert_eq!(*destination_name, None);
            fixture.assert_originals();
            assert_eq!(recovery.len(), 1);
            let receipt = &recovery[0];
            assert_eq!(receipt.role, RenameRecoveryRole::EmptyStage);
            let path = decode(&receipt.address);
            assert!(path.is_absolute());
            assert_eq!(address(&path), receipt.address);
            assert_eq!(receipt.identity, identity_at(&path).bytes());
            assert!(fs::symlink_metadata(&path).unwrap().is_dir());
            assert_eq!(fs::read_dir(&path).unwrap().count(), 0);
            assert_eq!(fixture.query(), result);
            fixture.close();
        }
    }

    #[test]
    fn unexpected_stage_child_survives_kernel_nonempty_cleanup_refusal() {
        let fixture = Fixture::new();
        let _hook = set_hook(move |point, path| {
            if point == Point::AfterStage {
                fs::write(path.join("unexpected-child"), b"preserve unexpected child").unwrap();
                return Err(injected());
            }
            Ok(())
        });
        let result = fixture.replace(&AtomicBool::new(false));
        let RenameOutcome::RecoveryRequired {
            source_location,
            destination_name,
            recovery,
            ..
        } = &result.outcome
        else {
            panic!("Nonempty stage must be retained with an explicit receipt: {result:?}");
        };
        assert_eq!(*source_location, RenameSourceLocation::OriginalName);
        assert_eq!(*destination_name, None);
        fixture.assert_originals();
        assert_eq!(recovery.len(), 1);
        let receipt = &recovery[0];
        assert_eq!(receipt.role, RenameRecoveryRole::EmptyStage);
        let stage = decode(&receipt.address);
        assert_eq!(receipt.identity, identity_at(&stage).bytes());
        assert_eq!(
            fs::read(stage.join("unexpected-child")).unwrap(),
            b"preserve unexpected child"
        );
        assert_eq!(fs::read_dir(&stage).unwrap().count(), 1);
        assert_eq!(fixture.query(), result);
        fixture.close();
    }

    #[test]
    fn failure_after_publication_reports_destination_and_never_restores_over_source() {
        let fixture = Fixture::new();
        let attempted_restore = Arc::new(AtomicBool::new(false));
        let hook_restore = attempted_restore.clone();
        let _hook = set_hook(move |point, _path| {
            if point == Point::AfterPublish {
                return Err(injected());
            }
            if point == Point::BeforeRestore {
                hook_restore.store(true, Ordering::Release);
            }
            Ok(())
        });
        let result = fixture.replace(&AtomicBool::new(false));
        let RenameOutcome::RecoveryRequired {
            source_location,
            destination_name,
            recovery,
            error,
        } = &result.outcome
        else {
            panic!("A post-commit failure must report actual source placement: {result:?}");
        };
        assert_eq!(*source_location, RenameSourceLocation::Destination);
        assert_eq!(destination_name, &Some(text(OsStr::new("target.bin"))));
        assert_eq!(error.code, ErrorCode::Io);
        assert!(!attempted_restore.load(Ordering::Acquire));
        assert!(!fixture.source.exists());
        assert_eq!(identity_at(&fixture.target), fixture.source_identity);
        assert_eq!(fs::read(&fixture.target).unwrap(), b"selected source");
        fixture.displaced(recovery);
        assert_eq!(fixture.query(), result);
        fixture.close();
    }

    fn uppercase_collision_fixture() -> (Fixture, RenameState) {
        let mut fixture = Fixture::new();
        let actual_target = fixture.temp.path.join("TARGET.bin");
        fs::rename(&fixture.target, &actual_target).unwrap();
        fixture.target = actual_target;
        let mut names: Vec<_> = fs::read_dir(&fixture.temp.path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [OsString::from("TARGET.bin"), OsString::from("source.bin")]
        );
        let conflict = state(fixture.client.call(Request::Rename {
            prepared: fixture.prepared.handle,
            attempt: 2,
            name: text(OsStr::new("target.bin")),
            policy: RenamePolicy::NoReplace,
        }));
        let RenameOutcome::Conflict(consent) = &conflict.outcome else {
            panic!("Expected fresh consent for the actual uppercase target: {conflict:?}");
        };
        assert!(consent.can_replace);
        assert_eq!(consent.target.identity, fixture.target_identity.bytes());
        fixture.consent = consent.clone();
        (fixture, conflict)
    }

    #[test]
    fn collision_disclosure_preserves_actual_target_basename_spelling() {
        let (fixture, conflict) = uppercase_collision_fixture();
        assert_eq!(
            fixture.consent.target.name,
            text(OsStr::new("TARGET.bin")),
            "Collision disclosure must preserve observed spelling independently of target.bin lookup"
        );
        fixture.assert_originals();
        assert_eq!(fixture.query(), conflict);
        fixture.close();
    }

    #[test]
    fn cancellation_after_capture_restores_actual_target_basename_spelling() {
        use std::sync::atomic::AtomicUsize;

        let (fixture, _) = uppercase_collision_fixture();
        // Do not assert the public conflict spelling here: this test must reach
        // target capture on the baseline and prove the separate restore defect.
        let cancellation = Arc::new(AtomicBool::new(false));
        let hook_cancel = cancellation.clone();
        let captures = Arc::new(AtomicUsize::new(0));
        let hook_captures = captures.clone();
        let restored = Arc::new(AtomicBool::new(false));
        let hook_restored = restored.clone();
        let _hook = set_hook(move |point, _path| {
            if point == Point::AfterCapture {
                hook_captures.fetch_add(1, Ordering::AcqRel);
                hook_cancel.store(true, Ordering::Release);
            }
            if point == Point::BeforeRestore {
                assert!(hook_cancel.load(Ordering::Acquire));
                hook_restored.store(true, Ordering::Release);
            }
            Ok(())
        });
        let request = || Request::Rename {
            prepared: fixture.prepared.handle,
            attempt: 3,
            name: text(OsStr::new("target.bin")),
            policy: RenamePolicy::RecoverableCurrentEntry {
                conflict: fixture.consent.token,
            },
        };
        let result = state(
            native::test_call(
                &fixture.client.api,
                fixture.client.instance,
                request(),
                &cancellation,
            )
            .unwrap(),
        );
        assert!(
            matches!(
                &result.outcome,
                RenameOutcome::Unchanged(Failure {
                    code: ErrorCode::Cancelled,
                    ..
                })
            ),
            "{result:?}"
        );
        assert_eq!(
            captures.load(Ordering::Acquire),
            1,
            "capture boundary must be reached"
        );
        assert!(
            restored.load(Ordering::Acquire),
            "restoration must be attempted"
        );
        fixture.assert_originals();
        let exact_names = || {
            let mut names: Vec<_> = fs::read_dir(&fixture.temp.path)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect();
            names.sort();
            names
        };
        let expected = [OsString::from("TARGET.bin"), OsString::from("source.bin")];
        assert_eq!(
            exact_names(),
            expected,
            "Cancellation must restore the original spelling, not requested target.bin"
        );
        assert_eq!(fixture.query(), result);
        assert_eq!(state(fixture.client.call(request())), result);
        assert_eq!(
            captures.load(Ordering::Acquire),
            1,
            "replay must reuse the retained receipt"
        );
        assert_eq!(exact_names(), expected);
        assert_eq!(
            fixture.client.cancelled(
                Request::CloseRename {
                    prepared: fixture.prepared.handle,
                },
                1,
            ),
            Response::Unit
        );
        assert_eq!(
            fixture.client.call(Request::CloseMount {
                mount: fixture.mount
            }),
            Response::Unit
        );
        fixture.assert_originals();
        assert_eq!(
            exact_names(),
            expected,
            "Close must not change restored names"
        );
    }
}
