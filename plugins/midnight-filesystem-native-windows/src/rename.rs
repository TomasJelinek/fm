//! Prepared native authority. The SDK owns protocol attempts and result retention.
use crate::{rename_native as native, rename_recovery};
use midnight_filesystem_plugin_api::*;
use midnight_filesystem_plugin_sdk::{
    CallContext,
    native::{NativeRename, PreparedRenameInfo, RENAME_RESOURCE_BYTES},
};
use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};
use windows_sys::Win32::{
    Foundation::{ERROR_LOCK_VIOLATION, ERROR_SHARING_VIOLATION},
    Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT,
};

pub(super) struct Prepared {
    pub(super) parent: File,
    // Keep this exact object alive across confirmation; no path repinning.
    _source: File,
    pub(super) parent_path: PathBuf,
    pub(super) source_path: PathBuf,
    pub(super) name: OsString,
    pub(super) source_snapshot: native::Snapshot,
    pub(super) parent_identity: native::Identity,
    info: PreparedRenameInfo,
    conflict: Mutex<Option<Target>>,
}
pub(super) struct Target {
    // Prevent file-ID recycling while its consent is outstanding.
    _file: File,
    pub(super) snapshot: native::Snapshot,
    // Exact request spelling remains the consent binding, including short aliases.
    requested_name: OsString,
    // Stored spelling of this opened entry, never another name inferred from FileId.
    pub(super) name: OsString,
}
impl Target {
    pub(super) fn retained_capacity(&self) -> usize {
        self.requested_name
            .capacity()
            .saturating_add(self.name.capacity())
            .saturating_add(self.snapshot.revision.capacity())
            .saturating_add(size_of::<Self>())
    }
}
pub(super) fn checked(context: &CallContext<'_>) -> Result<(), Failure> {
    if context.is_cancelled() {
        Err(Failure::new(
            ErrorCode::Cancelled,
            "Rename canceled before publication.",
        ))
    } else {
        Ok(())
    }
}
pub(super) fn changed() -> Failure {
    Failure::new(
        ErrorCode::SourceChanged,
        "The prepared source, parent or consented binding changed.",
    )
}
pub(super) fn io_failure(error: io::Error) -> Failure {
    let kind = error.kind();
    let mut failure = Failure::from_io(error);
    if kind == io::ErrorKind::OutOfMemory {
        failure.code = ErrorCode::ResourceLimit;
    }
    if kind == io::ErrorKind::Interrupted {
        failure.code = ErrorCode::Cancelled;
    }
    if failure.os_code.is_some_and(|code| {
        [ERROR_SHARING_VIOLATION as i32, ERROR_LOCK_VIOLATION as i32].contains(&code)
    }) {
        failure.code = ErrorCode::Busy;
    }
    failure
}
fn binding_error(error: io::Error) -> Failure {
    if error.kind() == io::ErrorKind::NotFound {
        changed()
    } else {
        io_failure(error)
    }
}
pub(super) fn text(name: &OsStr) -> NativeText {
    NativeText {
        encoding: NameEncoding::WindowsUtf16Le,
        bytes: name.encode_wide().flat_map(u16::to_le_bytes).collect(),
    }
}
#[cfg(test)]
pub(super) fn address(path: &Path) -> Address {
    Address {
        encoding: AddressEncoding::WindowsUtf16Le,
        bytes: path
            .as_os_str()
            .encode_wide()
            .flat_map(u16::to_le_bytes)
            .collect(),
    }
}
fn destination(name: &NativeText) -> Result<OsString, Failure> {
    if name.encoding != NameEncoding::WindowsUtf16Le {
        return Err(Failure::new(
            ErrorCode::Unsupported,
            "Native Windows Rename requires UTF-16LE names.",
        ));
    }
    name.validate_rename_basename().map_err(|error| {
        Failure::new(
            if error == WireError::Limit {
                ErrorCode::ResourceLimit
            } else {
                ErrorCode::Invalid
            },
            "A valid Windows destination basename is required.",
        )
    })?;
    let (pairs, remainder) = name.bytes.as_chunks::<2>();
    if !remainder.is_empty() {
        return Err(Failure::new(
            ErrorCode::Invalid,
            "Invalid UTF-16LE basename.",
        ));
    }
    let units: Vec<_> = pairs.iter().copied().map(u16::from_le_bytes).collect();
    Ok(OsString::from_wide(&units))
}
fn ordinary_file(snapshot: &native::Snapshot) -> bool {
    snapshot.kind == Kind::File
        && snapshot
            .metadata
            .native_attributes
            .is_some_and(|attributes| attributes & FILE_ATTRIBUTE_REPARSE_POINT == 0)
}

pub(super) fn prepare(
    path: &Path,
    context: &CallContext<'_>,
) -> Result<Arc<dyn NativeRename>, Failure> {
    checked(context)?;
    // Bound all retained/path/receipt capacities before native opens; the SDK has
    // already reserved this operation's fixed allocation charge and slot.
    if path
        .as_os_str()
        .len()
        .saturating_mul(6)
        .saturating_add(MAX_RENAME_RECEIPT_BYTES)
        > RENAME_RESOURCE_BYTES
    {
        return Err(Failure::new(
            ErrorCode::ResourceLimit,
            "Native Rename path/receipt capacity exceeds its reservation.",
        ));
    }
    let path = crate::path::literal(path)?;
    for component in path.components() {
        if let Component::Normal(name) = component
            && name
                .encode_wide()
                .any(|unit| unit == 0 || unit == u16::from(b':'))
        {
            return Err(Failure::new(
                ErrorCode::Unsupported,
                "Streams and special native namespace syntax cannot be renamed.",
            ));
        }
    }
    let name = path
        .file_name()
        .ok_or_else(|| {
            Failure::new(
                ErrorCode::Unsupported,
                "A non-root native entry is required.",
            )
        })?
        .to_owned();
    text(&name)
        .validate_rename_observed_basename()
        .map_err(|_| Failure::new(ErrorCode::Invalid, "Invalid observed native basename."))?;
    let parent_path = path
        .parent()
        .ok_or_else(|| {
            Failure::new(
                ErrorCode::Unsupported,
                "Rename requires an immediate parent.",
            )
        })?
        .to_owned();
    let parent = native::open_parent(&parent_path).map_err(io_failure)?;
    let parent_identity = native::identity(&parent).map_err(io_failure)?;
    let source = native::open_child(&parent, &name, native::Access::Observe).map_err(io_failure)?;
    let source_snapshot = native::snapshot(&source)?;
    if source_snapshot.kind == Kind::Other {
        return Err(Failure::new(
            ErrorCode::Unsupported,
            "This native object kind cannot be renamed.",
        ));
    }
    let mut revision = b"native.windows.rename.binding.v1".to_vec();
    revision.extend(parent_identity.bytes());
    revision.extend(source_snapshot.identity.bytes());
    revision.push(source_snapshot.kind as u8);
    let info = PreparedRenameInfo {
        source: RenameObject {
            identity: source_snapshot.identity.bytes(),
            revision,
            name: text(&name),
            kind: source_snapshot.kind,
            metadata: source_snapshot.metadata.clone(),
        },
        parent_identity: parent_identity.bytes(),
        recoverable_replace: ordinary_file(&source_snapshot) && native::local_fixed_ntfs(&parent),
    };
    let prepared = Prepared {
        parent,
        _source: source,
        parent_path,
        source_path: path,
        name,
        source_snapshot,
        parent_identity,
        info,
        conflict: Mutex::new(None),
    };
    // Final successful binding validation seals the current object pin. It is
    // not evidence about the object that occupied this name at Browse time.
    prepared.revalidate(false)?;
    checked(context)?;
    Ok(Arc::new(prepared))
}
impl Prepared {
    pub(super) fn retained_capacity(&self) -> usize {
        self.source_path
            .capacity()
            .saturating_add(self.parent_path.capacity())
            .saturating_add(self.name.capacity())
            .saturating_add(self.source_snapshot.revision.capacity())
            .saturating_add(self.info.source.identity.capacity())
            .saturating_add(self.info.source.revision.capacity())
            .saturating_add(self.info.source.name.bytes.capacity())
            .saturating_add(self.info.parent_identity.capacity())
            .saturating_add(size_of::<Self>())
    }
    pub(super) fn validate_parent(&self) -> Result<(), Failure> {
        let visible = native::open_parent(&self.parent_path).map_err(binding_error)?;
        if native::identity(&visible).map_err(io_failure)? != self.parent_identity {
            return Err(changed());
        }
        Ok(())
    }
    pub(super) fn revalidate(&self, exclusive: bool) -> Result<File, Failure> {
        self.validate_parent()?;
        let file = native::open_child(
            &self.parent,
            &self.name,
            if exclusive {
                native::Access::SourceCommit
            } else {
                native::Access::Observe
            },
        )
        .map_err(binding_error)?;
        let identity = native::identity(&file).map_err(io_failure)?;
        let metadata = file.metadata().map_err(io_failure)?;
        let kind = crate::adapter::info(metadata.file_type(), Some(&metadata)).kind;
        // Rename authority guards object/kind/binding, not a content-reader
        // revision. Concurrent data edits do not substitute this retained object.
        if identity != self.source_snapshot.identity || kind != self.source_snapshot.kind {
            return Err(changed());
        }
        Ok(file)
    }
    fn observed_target_name(
        &self,
        file: &File,
        retained_bytes: usize,
    ) -> Result<OsString, Failure> {
        let scratch_bytes = self
            .retained_capacity()
            .checked_add(retained_bytes)
            .and_then(|bytes| bytes.checked_add(MAX_RENAME_RECEIPT_BYTES))
            .and_then(|bytes| RENAME_RESOURCE_BYTES.checked_sub(bytes))
            .ok_or_else(|| {
                Failure::new(
                    ErrorCode::ResourceLimit,
                    "Rename name buffers exceed the reserved budget.",
                )
            })?;
        let name = native::observed_child_basename(file, &self.parent, scratch_bytes).map_err(
            |error| {
                if error.kind() == io::ErrorKind::InvalidData {
                    changed()
                } else {
                    io_failure(error)
                }
            },
        )?;
        let bytes = name.encode_wide().count().checked_mul(2).ok_or_else(|| {
            Failure::new(
                ErrorCode::ResourceLimit,
                "Observed Rename name is too large.",
            )
        })?;
        if name
            .capacity()
            .checked_add(bytes)
            .is_none_or(|size| size > scratch_bytes)
        {
            return Err(Failure::new(
                ErrorCode::ResourceLimit,
                "Observed Rename name exceeds its reservation.",
            ));
        }
        let mut encoded = Vec::new();
        encoded.try_reserve_exact(bytes).map_err(|_| {
            Failure::new(
                ErrorCode::ResourceLimit,
                "Observed Rename name allocation failed.",
            )
        })?;
        if name
            .capacity()
            .checked_add(encoded.capacity())
            .is_none_or(|size| size > scratch_bytes)
        {
            return Err(Failure::new(
                ErrorCode::ResourceLimit,
                "Observed Rename name exceeds its reservation.",
            ));
        }
        encoded.extend(name.encode_wide().flat_map(u16::to_le_bytes));
        NativeText {
            encoding: NameEncoding::WindowsUtf16Le,
            bytes: encoded,
        }
        .validate_rename_observed_basename()
        .map_err(|_| changed())?;
        Ok(name)
    }
    fn inspect(&self, name: &OsStr, retained: Option<&Target>) -> Result<Option<Target>, Failure> {
        self.validate_parent()?;
        let file = match native::open_child(&self.parent, name, native::Access::Observe) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io_failure(error)),
        };
        let snapshot = native::snapshot(&file)?;
        if snapshot.kind == Kind::Other {
            return Err(Failure::new(
                ErrorCode::Unsupported,
                "The collision object kind cannot be represented safely.",
            ));
        }
        let requested_name = name.to_owned();
        let retained_bytes = retained
            .map_or(0, Target::retained_capacity)
            .saturating_add(requested_name.capacity())
            .saturating_add(snapshot.revision.capacity())
            .saturating_add(MAX_REVISION)
            .saturating_add(size_of::<native::Snapshot>())
            .saturating_add(size_of::<Target>());
        let observed_name = self.observed_target_name(&file, retained_bytes)?;
        let visible = native::open_child(&self.parent, name, native::Access::Observe)
            .map_err(binding_error)?;
        let observed = native::open_child(&self.parent, &observed_name, native::Access::Observe)
            .map_err(binding_error)?;
        let observed_snapshot = native::snapshot(&observed)?;
        if native::identity(&visible).map_err(io_failure)? != snapshot.identity
            || observed_snapshot.identity != snapshot.identity
            || observed_snapshot.revision != snapshot.revision
            || native::snapshot(&file)?.revision != snapshot.revision
        {
            return Err(changed());
        }
        Ok(Some(Target {
            _file: file,
            snapshot,
            requested_name,
            name: observed_name,
        }))
    }
    fn conflict_outcome(&self, target: Target, retained: &mut Option<Target>) -> RenameOutcome {
        let outcome = RenameOutcome::Conflict(RenameConflict {
            token: 0, // SDK assigns the non-reused, prepared-handle-scoped token.
            target: RenameObject {
                identity: target.snapshot.identity.bytes(),
                revision: target.snapshot.revision.clone(),
                name: text(&target.name),
                kind: target.snapshot.kind,
                metadata: target.snapshot.metadata.clone(),
            },
            can_replace: self.info.recoverable_replace
                && ordinary_file(&target.snapshot)
                && target.snapshot.identity != self.source_snapshot.identity
                && native::local_fixed_ntfs(&self.parent),
        });
        *retained = Some(target);
        outcome
    }
    fn refresh_conflict(&self, name: &OsStr, retained: &mut Option<Target>) -> RenameOutcome {
        match self.inspect(name, retained.as_ref()) {
            Ok(Some(target)) => self.conflict_outcome(target, retained),
            Ok(None) => RenameOutcome::Unchanged(changed()),
            Err(error) => RenameOutcome::Unchanged(error),
        }
    }
    fn is_case_of_original(
        &self,
        target: &Target,
        name: &OsStr,
        context: &CallContext<'_>,
    ) -> Result<bool, Failure> {
        if target.snapshot.identity != self.source_snapshot.identity
            || !native::same_name_ignore_case(name, &self.name).map_err(io_failure)?
        {
            return Ok(false);
        }
        if target.snapshot.links == 1 {
            return Ok(true);
        }
        // A file ID identifies an object, not its individual hard-link names.
        // Only this ambiguous case needs bounded enumeration, never content I/O.
        let directory = native::open_enumeration(&self.parent_path).map_err(io_failure)?;
        if native::identity(&directory).map_err(io_failure)? != self.parent_identity {
            return Err(changed());
        }
        native::has_exact_name(&directory, name, &|| context.is_cancelled())
            .map(|found| !found)
            .map_err(io_failure)
    }
    fn no_replace(
        &self,
        source: &File,
        name: &OsStr,
        context: &CallContext<'_>,
        retained: &mut Option<Target>,
    ) -> RenameOutcome {
        if name == self.name {
            return RenameOutcome::Renamed {
                name: text(name),
                same_entry: true,
                recovery: vec![],
                durability_confirmed: false,
            };
        }
        match self.inspect(name, retained.as_ref()) {
            Ok(Some(target)) => match self.is_case_of_original(&target, name, context) {
                Ok(true) => {}
                Ok(false) => return self.conflict_outcome(target, retained),
                Err(error) => return RenameOutcome::Unchanged(error),
            },
            Ok(None) => {}
            Err(error) => return RenameOutcome::Unchanged(error),
        }
        let mut request = match native::RenameBuffer::new(name) {
            Ok(request) => request,
            Err(error) => return RenameOutcome::Unchanged(io_failure(error)),
        };
        let destination = self.parent_path.join(name);
        if let Err(error) = point(Point::BeforePublish, &destination)
            .and_then(|()| checked(context))
            .and_then(|()| self.revalidate(false).map(|_| ()))
        {
            return RenameOutcome::Unchanged(error);
        }
        // Reinspect after the controlled commit boundary too; native no-replace
        // remains authoritative for distinct late arrivals after this check.
        match self.inspect(name, retained.as_ref()) {
            Ok(Some(target)) => match self.is_case_of_original(&target, name, context) {
                Ok(true) => {}
                Ok(false) => return self.conflict_outcome(target, retained),
                Err(error) => return RenameOutcome::Unchanged(error),
            },
            Ok(None) => {}
            Err(error) => return RenameOutcome::Unchanged(error),
        }
        if let Err(error) = checked(context) {
            return RenameOutcome::Unchanged(error);
        }
        if let Err(error) = request.apply(source, Some(&self.parent)) {
            if error.kind() == io::ErrorKind::AlreadyExists {
                return self.refresh_conflict(name, retained);
            }
            return RenameOutcome::Unchanged(io_failure(error));
        }
        if let Err(error) = point(Point::AfterPublish, &destination) {
            return RenameOutcome::RecoveryRequired {
                source_location: RenameSourceLocation::Destination,
                destination_name: Some(text(name)),
                recovery: vec![],
                error,
            };
        }
        RenameOutcome::Renamed {
            name: text(name),
            same_entry: false,
            recovery: vec![],
            durability_confirmed: false,
        }
    }
    fn replace(
        &self,
        source: File,
        name: &OsStr,
        context: &CallContext<'_>,
        retained: &mut Option<Target>,
    ) -> RenameOutcome {
        let Some(expected) = retained.as_ref() else {
            return RenameOutcome::Unchanged(Failure::new(
                ErrorCode::Invalid,
                "No native target consent is retained.",
            ));
        };
        if expected.requested_name != name {
            return RenameOutcome::Unchanged(Failure::new(
                ErrorCode::Invalid,
                "The native destination differs from retained consent.",
            ));
        }
        let target = match native::open_child(&self.parent, name, native::Access::TargetCommit) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return self.refresh_conflict(name, retained);
            }
            Err(error) => {
                // A formerly consented file can become a directory/link. The
                // commit-only open rejects that kind before an ID comparison;
                // observe it without commit rights to disclose fresh consent.
                return match self.inspect(name, retained.as_ref()) {
                    Ok(Some(current))
                        if current.snapshot.identity != expected.snapshot.identity
                            || current.snapshot.revision != expected.snapshot.revision =>
                    {
                        self.conflict_outcome(current, retained)
                    }
                    Ok(None) => RenameOutcome::Unchanged(changed()),
                    _ => RenameOutcome::Unchanged(io_failure(error)),
                };
            }
        };
        let snapshot = match native::snapshot(&target) {
            Ok(snapshot) => snapshot,
            Err(error) => return RenameOutcome::Unchanged(error),
        };
        if snapshot.identity != expected.snapshot.identity
            || snapshot.revision != expected.snapshot.revision
        {
            drop(target);
            return self.refresh_conflict(name, retained);
        }
        let current_name = match self.observed_target_name(
            &target,
            expected
                .retained_capacity()
                .saturating_add(snapshot.revision.capacity())
                .saturating_add(size_of::<native::Snapshot>()),
        ) {
            Ok(name) => name,
            Err(error) => return RenameOutcome::Unchanged(error),
        };
        if current_name != expected.name {
            drop(target);
            return self.refresh_conflict(name, retained);
        }
        drop(current_name);
        let current_source = match source.metadata() {
            Ok(metadata) => metadata,
            Err(error) => return RenameOutcome::Unchanged(io_failure(error)),
        };
        let source_attributes =
            crate::adapter::info(current_source.file_type(), Some(&current_source));
        if !self.info.recoverable_replace
            || source_attributes.kind != Kind::File
            || source_attributes
                .metadata
                .native_attributes
                .is_none_or(|attributes| attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0)
            || !ordinary_file(&snapshot)
            || snapshot.identity == self.source_snapshot.identity
            || !native::local_fixed_ntfs(&self.parent)
        {
            return RenameOutcome::Unchanged(Failure::new(
                ErrorCode::Unsupported,
                "Recoverable replacement requires distinct ordinary files on supported local fixed NTFS.",
            ));
        }
        if let Err(error) = checked(context).and_then(|()| self.revalidate(false).map(|_| ())) {
            return RenameOutcome::Unchanged(error);
        }
        rename_recovery::replace(self, source, target, expected, name, context)
    }
}
impl NativeRename for Prepared {
    fn prepared(&self) -> PreparedRenameInfo {
        self.info.clone()
    }
    fn execute(
        &self,
        name: &NativeText,
        policy: &RenamePolicy,
        context: &CallContext<'_>,
    ) -> RenameOutcome {
        if let Err(error) = checked(context) {
            return RenameOutcome::Unchanged(error);
        }
        let name = match destination(name) {
            Ok(name) => name,
            Err(error) => return RenameOutcome::Unchanged(error),
        };
        let source = match self.revalidate(true) {
            Ok(source) => source,
            Err(error) => return RenameOutcome::Unchanged(error),
        };
        let mut retained = self
            .conflict
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        match policy {
            RenamePolicy::NoReplace => {
                drop(retained.take());
                self.no_replace(&source, &name, context, &mut retained)
            }
            RenamePolicy::RecoverableCurrentEntry { .. } => {
                self.replace(source, &name, context, &mut retained)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Point {
    AfterStage,
    AfterCapture,
    BeforePublish,
    AfterPublish,
    BeforeRestore,
    BeforeCleanup,
}
pub(super) fn point(point: Point, path: &Path) -> Result<(), Failure> {
    #[cfg(test)]
    {
        tests::run_hook(point, path)
    }
    #[cfg(not(test))]
    {
        let _ = (point, path);
        Ok(())
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use std::cell::RefCell;
    type Hook = Box<dyn FnMut(Point, &Path) -> Result<(), Failure>>;
    thread_local! { static HOOK: RefCell<Option<Hook>> = const { RefCell::new(None) }; }
    pub(crate) struct HookReset(Option<Hook>);
    impl Drop for HookReset {
        fn drop(&mut self) {
            HOOK.with(|hook| *hook.borrow_mut() = self.0.take());
        }
    }
    pub(crate) fn set_hook(
        hook: impl FnMut(Point, &Path) -> Result<(), Failure> + 'static,
    ) -> HookReset {
        HOOK.with(|current| HookReset(current.borrow_mut().replace(Box::new(hook))))
    }
    pub(super) fn run_hook(point: Point, path: &Path) -> Result<(), Failure> {
        HOOK.with(|hook| {
            if let Some(hook) = hook.borrow_mut().as_mut() {
                hook(point, path)
            } else {
                Ok(())
            }
        })
    }
    use crate::test_support::{Client, Temp};
    use std::{
        fs,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };
    fn pin(client: &Client, path: &Path) -> (u64, PreparedRename) {
        let (mount, node) = client.resolve(path);
        let Response::RenamePrepared(prepared) = client.call(Request::PrepareRename {
            mount,
            node: node.id,
        }) else {
            panic!("native preparation")
        };
        (mount, prepared)
    }
    fn rename_request(prepared: u64, name: &str) -> Request {
        Request::Rename {
            prepared,
            attempt: 1,
            name: text(OsStr::new(name)),
            policy: RenamePolicy::NoReplace,
        }
    }
    fn state(response: Response) -> RenameState {
        let Response::RenameState(state) = response else {
            panic!("native retained state: {response:?}")
        };
        state
    }
    fn close(client: &Client, mount: u64, prepared: u64) {
        assert_eq!(
            client.call(Request::CloseRename { prepared }),
            Response::Unit
        );
        assert_eq!(client.call(Request::CloseMount { mount }), Response::Unit);
    }
    fn identity_at(path: &Path) -> native::Identity {
        let parent = native::open_parent(path.parent().unwrap()).unwrap();
        let file = native::open_child(&parent, path.file_name().unwrap(), native::Access::Observe)
            .unwrap();
        native::identity(&file).unwrap()
    }
    #[test]
    fn late_distinct_destination_is_a_fresh_conflict_without_source_publication() {
        let temp = Temp::new();
        let source = temp.path.join("source");
        let destination = temp.path.join("destination");
        fs::write(&source, b"source bytes").unwrap();
        let before = identity_at(&source);
        let client = Client::new();
        let (mount, prepared) = pin(&client, &source);
        let arrived = destination.clone();
        let _hook = set_hook(move |point, _| {
            if point == Point::BeforePublish {
                fs::write(&arrived, b"late winner").unwrap();
            }
            Ok(())
        });
        let result = state(client.call(rename_request(prepared.handle, "destination")));
        let RenameOutcome::Conflict(conflict) = &result.outcome else {
            panic!("late destination conflict: {result:?}")
        };
        assert_eq!(conflict.target.identity, identity_at(&destination).bytes());
        assert_ne!(identity_at(&destination), before);
        assert_eq!(identity_at(&source), before);
        assert_eq!(fs::read(&source).unwrap(), b"source bytes");
        assert_eq!(fs::read(&destination).unwrap(), b"late winner");
        assert_eq!(fs::read_dir(&temp.path).unwrap().count(), 2);
        assert_eq!(
            client.call(Request::GetRenameState {
                prepared: prepared.handle
            }),
            Response::RenameState(result)
        );
        close(&client, mount, prepared.handle);
    }
    #[test]
    fn no_replace_late_cancel_reports_committed_identity_and_replays_once() {
        let temp = Temp::new();
        let source = temp.path.join("source");
        let destination = temp.path.join("destination");
        fs::write(&source, b"committed source").unwrap();
        let before = identity_at(&source);
        let client = Client::new();
        let (mount, prepared) = pin(&client, &source);
        let cancel = Arc::new(AtomicBool::new(false));
        let after_commit = cancel.clone();
        let _hook = set_hook(move |point, _| {
            if point == Point::AfterPublish {
                after_commit.store(true, Ordering::Release);
            }
            Ok(())
        });
        let request = rename_request(prepared.handle, "destination");
        let result =
            native::test_call(&client.api, client.instance, request.clone(), &cancel).unwrap();
        assert!(matches!(
            state(result.clone()).outcome,
            RenameOutcome::Renamed {
                same_entry: false,
                ..
            }
        ));
        assert!(cancel.load(Ordering::Acquire));
        assert!(!source.exists());
        assert_eq!(identity_at(&destination), before);
        assert_eq!(fs::read(&destination).unwrap(), b"committed source");
        assert_eq!(
            native::test_call(
                &client.api,
                client.instance,
                Request::GetRenameState {
                    prepared: prepared.handle
                },
                &cancel
            )
            .unwrap(),
            result
        );
        assert_eq!(client.call(request), result);
        close(&client, mount, prepared.handle);
    }
    #[test]
    fn file_and_directory_rename_require_metadata_delete_rights_not_content_or_listing() {
        for directory in [false, true] {
            let temp = Temp::new();
            let source = temp.path.join("selected");
            let destination = temp.path.join("renamed");
            if directory {
                fs::create_dir(&source).unwrap();
            } else {
                fs::write(&source, b"no content read privilege").unwrap();
            }
            let before = identity_at(&source);
            let access = native::test_metadata_only(&source).unwrap();
            let denied = if directory {
                fs::read_dir(&source).err().unwrap()
            } else {
                fs::File::open(&source).err().unwrap()
            };
            assert_eq!(
                denied.kind(),
                io::ErrorKind::PermissionDenied,
                "fixture must actually deny data/list access"
            );
            let client = Client::new();
            let (mount, prepared) = pin(&client, &source);
            assert_eq!(prepared.source.identity, before.bytes());
            let result = state(client.call(rename_request(prepared.handle, "renamed")));
            assert!(matches!(
                result.outcome,
                RenameOutcome::Renamed {
                    same_entry: false,
                    ..
                }
            ));
            assert!(!source.exists());
            assert_eq!(identity_at(&destination), before);
            let still_denied = if directory {
                fs::read_dir(&destination).err().unwrap()
            } else {
                fs::File::open(&destination).err().unwrap()
            };
            assert_eq!(still_denied.kind(), io::ErrorKind::PermissionDenied);
            close(&client, mount, prepared.handle);
            access.restore().unwrap();
            if directory {
                assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
            } else {
                assert_eq!(
                    fs::read(&destination).unwrap(),
                    b"no content read privilege"
                );
            }
        }
    }
    #[test]
    fn source_data_edits_do_not_replace_the_prepared_object_authority() {
        let temp = Temp::new();
        let source = temp.path.join("source");
        fs::write(&source, b"old source bytes").unwrap();
        let before = identity_at(&source);
        let client = Client::new();
        let (mount, prepared) = pin(&client, &source);
        fs::write(&source, b"edited same source object").unwrap();
        assert_eq!(identity_at(&source), before);
        let result = state(client.call(rename_request(prepared.handle, "renamed")));
        assert!(matches!(result.outcome, RenameOutcome::Renamed { .. }));
        assert_eq!(identity_at(&temp.path.join("renamed")), before);
        assert_eq!(
            fs::read(temp.path.join("renamed")).unwrap(),
            b"edited same source object"
        );
        close(&client, mount, prepared.handle);
    }
}
