//! Prepared same-parent Rename. Pins define authority; live namespace checks
//! are not inode-CAS or isolation from the same UID/root and preopened descendants.
use crate::{rename_recovery::Stage, rename_sys as sys, system};
use midnight_filesystem_plugin_api::*;
use midnight_filesystem_plugin_sdk::{
    CallContext,
    native::{NativeRename, PreparedRenameInfo, RENAME_RESOURCE_BYTES},
};
use std::{
    ffi::CString,
    fs::{File, Metadata as NativeMetadata},
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
pub(crate) const ITEM: &std::ffi::CStr = c"item";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Identity {
    pub(crate) device: u64,
    pub(crate) inode: u64,
    kind: u32,
}
impl Identity {
    pub(crate) fn of(file: &File) -> Result<Self, Failure> {
        Ok(Self::metadata(&file.metadata().map_err(Failure::from_io)?))
    }
    fn metadata(metadata: &NativeMetadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            kind: metadata.mode() & libc::S_IFMT,
        }
    }
    pub(crate) fn write_bytes(self, bytes: &mut Vec<u8>) {
        // Receipt buffers reserve these twenty bytes before namespace writes.
        debug_assert!(bytes.capacity() >= 20);
        bytes.clear();
        bytes.extend_from_slice(&self.device.to_le_bytes());
        bytes.extend_from_slice(&self.inode.to_le_bytes());
        bytes.extend_from_slice(&self.kind.to_le_bytes());
    }
    pub(crate) fn bytes(self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(20);
        self.write_bytes(&mut bytes);
        bytes
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Revision {
    size: u64,
    modified: (i64, i64),
    changed: (i64, i64),
    mode: u32,
    uid: u32,
    gid: u32,
    links: u64,
}
impl Revision {
    pub(crate) fn metadata(value: &NativeMetadata) -> Self {
        Self {
            size: value.len(),
            modified: (value.mtime(), value.mtime_nsec()),
            changed: (value.ctime(), value.ctime_nsec()),
            mode: value.mode(),
            uid: value.uid(),
            gid: value.gid(),
            links: value.nlink(),
        }
    }
    pub(crate) fn of(file: &File) -> Result<Self, Failure> {
        Ok(Self::metadata(&file.metadata().map_err(Failure::from_io)?))
    }
    pub(crate) fn after_rename_matches(self, current: Self) -> bool {
        // Exchange itself updates ctime. It is compared in full before exchange,
        // then excluded only from this post-exchange check, as in audited host Rename.
        self.size == current.size
            && self.modified == current.modified
            && self.mode == current.mode
            && self.uid == current.uid
            && self.gid == current.gid
            && self.links == current.links
    }
    fn bytes(self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(64);
        for value in [
            self.size,
            self.modified.0 as u64,
            self.modified.1 as u64,
            self.changed.0 as u64,
            self.changed.1 as u64,
            self.links,
        ] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for value in [self.mode, self.uid, self.gid] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes
    }
}
pub(crate) fn changed(message: &'static str) -> Failure {
    Failure::new(ErrorCode::SourceChanged, message)
}
pub(crate) fn limit() -> Failure {
    Failure::new(
        ErrorCode::ResourceLimit,
        "Exact Rename state and recovery receipt exceed the reserved resource budget.",
    )
}
pub(crate) fn trusted_parent(parent: &File) -> Result<(), Failure> {
    let metadata = parent.metadata().map_err(Failure::from_io)?;
    let uid = sys::effective_uid();
    if !metadata.is_dir()
        || (metadata.uid() != uid && metadata.uid() != 0)
        || (metadata.mode() & 0o022 != 0 && metadata.mode() & 0o1000 == 0)
    {
        return Err(Failure::new(
            ErrorCode::Unsupported,
            "Rename recovery requires an owned or root-owned trusted sticky parent.",
        ));
    }
    Ok(())
}
pub(crate) fn local_filesystem(parent: &File) -> bool {
    system::filesystem(parent).is_ok_and(|value| {
        matches!(
            value.f_type as u64,
            0xef53 | 0x5846_5342 | 0x9123_683e | 0x0102_1994
        )
    })
}
fn qualified(parent: &File, source: Identity) -> bool {
    Identity::of(parent).is_ok_and(|identity| identity.device == source.device)
        && trusted_parent(parent).is_ok()
        && local_filesystem(parent)
}
fn budget(parent: usize, source: usize, destination: usize) -> Result<(), Failure> {
    // Pessimistic simultaneous native path/receipt/conflict capacities plus SDK
    // receipt, input/result clones and fixed bookkeeping. Public ceilings are independent.
    let total = parent
        .saturating_add(96)
        .saturating_mul(12)
        .saturating_add(source.saturating_mul(6))
        .saturating_add(destination.saturating_mul(5))
        .saturating_add(MAX_RENAME_RECEIPT_BYTES)
        .saturating_add(64 * 1024);
    let addresses = parent.saturating_add(102).saturating_mul(3);
    if total > RENAME_RESOURCE_BYTES || addresses > MAX_RENAME_RECOVERY_ADDRESS_BYTES {
        return Err(limit());
    }
    Ok(())
}
struct Target {
    name: CString,
    file: File,
    identity: Identity,
    revision: Revision,
    object: RenameObject,
}
impl Target {
    fn capture(parent: &File, name: &CString) -> Result<Option<Self>, Failure> {
        let file = match sys::pin(parent, name) {
            Ok(value) => value,
            Err(failure) if failure.code == ErrorCode::NotFound => return Ok(None),
            Err(failure) => return Err(failure),
        };
        let before = Revision::of(&file)?;
        let metadata = file.metadata().map_err(Failure::from_io)?;
        let info = crate::adapter::info(&metadata);
        if info.kind == Kind::Other {
            return Err(Failure::new(
                ErrorCode::Unsupported,
                "Unsupported collision object kind.",
            ));
        }
        let identity = Identity::metadata(&metadata);
        let revision = Revision::metadata(&metadata);
        if before != revision
            || Revision::of(&file)? != revision
            || Identity::of(&sys::pin(parent, name)?)? != identity
        {
            return Err(changed(
                "Destination changed while capturing conflict metadata.",
            ));
        }
        Ok(Some(Self {
            object: RenameObject {
                identity: identity.bytes(),
                revision: revision.bytes(),
                name: NativeText {
                    encoding: NameEncoding::UnixBytes,
                    bytes: name.as_bytes().to_vec(),
                },
                kind: info.kind,
                metadata: info.metadata,
            },
            name: name.clone(),
            file,
            identity,
            revision,
        }))
    }
    fn current(&self, parent: &File, name: &CString) -> Result<(), Failure> {
        if self.name != *name {
            return Err(changed("Destination name differs from retained consent."));
        }
        let visible = sys::pin(parent, name)?;
        if Identity::of(&visible)? != self.identity
            || Identity::of(&self.file)? != self.identity
            || Revision::of(&visible)? != self.revision
            || Revision::of(&self.file)? != self.revision
        {
            return Err(changed(
                "Consented destination identity or revision changed.",
            ));
        }
        Ok(())
    }
}
pub(crate) struct Prepared {
    pub(crate) parent_path: PathBuf,
    pub(crate) name: CString,
    pub(crate) parent: File,
    pub(crate) parent_identity: Identity,
    pub(crate) source: File,
    pub(crate) identity: Identity,
    info: PreparedRenameInfo,
    conflict: Mutex<Option<Target>>,
}
pub(crate) fn prepare(
    path: &Path,
    context: &CallContext<'_>,
) -> Result<Arc<dyn NativeRename>, Failure> {
    system::cancel(context)?;
    let raw = path.as_os_str().as_bytes();
    if !path.is_absolute()
        || raw.contains(&0)
        || raw
            .split(|byte| *byte == b'/')
            .any(|part| part == b"." || part == b"..")
    {
        return Err(Failure::new(
            ErrorCode::Invalid,
            "Rename requires an absolute native path without dot components.",
        ));
    }
    let name = path.file_name().ok_or_else(|| {
        Failure::new(
            ErrorCode::Unsupported,
            "Filesystem roots cannot be renamed.",
        )
    })?;
    let parent_path = path.parent().ok_or_else(|| {
        Failure::new(
            ErrorCode::Unsupported,
            "Rename requires an immediate parent.",
        )
    })?;
    budget(
        parent_path.as_os_str().as_bytes().len(),
        name.as_bytes().len(),
        MAX_RENAME_NAME_BYTES,
    )?;
    let name = CString::new(name.as_bytes())
        .map_err(|_| Failure::new(ErrorCode::Invalid, "NUL in source name."))?;
    let parent = system::open_path(parent_path, libc::O_PATH | libc::O_DIRECTORY, context)?;
    let parent_identity = Identity::of(&parent)?;
    let source = sys::pin(&parent, &name)?;
    let metadata = source.metadata().map_err(Failure::from_io)?;
    let identity = Identity::metadata(&metadata);
    let info = crate::adapter::info(&metadata);
    if info.kind == Kind::Other
        || identity.device != parent_identity.device
        || identity == parent_identity
    {
        return Err(Failure::new(
            ErrorCode::Unsupported,
            "Unsupported source kind or mount root.",
        ));
    }
    let prepared = Prepared {
        info: PreparedRenameInfo {
            source: RenameObject {
                identity: identity.bytes(),
                revision: Revision::metadata(&metadata).bytes(),
                name: NativeText {
                    encoding: NameEncoding::UnixBytes,
                    bytes: name.as_bytes().to_vec(),
                },
                kind: info.kind,
                metadata: info.metadata,
            },
            parent_identity: parent_identity.bytes(),
            recoverable_replace: info.kind == Kind::File && qualified(&parent, identity),
        },
        parent_path: parent_path.to_owned(),
        name,
        parent,
        parent_identity,
        source,
        identity,
        conflict: Mutex::new(None),
    };
    prepared.revalidate()?;
    system::cancel(context)?;
    Ok(Arc::new(prepared))
}
impl Prepared {
    pub(crate) fn check_parent(&self) -> Result<(), Failure> {
        let visible = sys::parent_path(&self.parent_path)
            .map_err(|_| changed("The actual parent binding changed."))?;
        if Identity::of(&visible)? != self.parent_identity
            || Identity::of(&self.parent)? != self.parent_identity
        {
            return Err(changed("The actual parent directory changed."));
        }
        Ok(())
    }
    pub(crate) fn revalidate(&self) -> Result<(), Failure> {
        self.check_parent()?;
        let current = sys::pin(&self.parent, &self.name)
            .map_err(|_| changed("The source binding changed."))?;
        if Identity::of(&current)? != self.identity || Identity::of(&self.source)? != self.identity
        {
            return Err(changed(
                "The pinned source no longer occupies its original name.",
            ));
        }
        Ok(())
    }
    fn observe_conflict(
        &self,
        name: &CString,
        retained: &mut Option<Target>,
    ) -> Result<RenameOutcome, Failure> {
        self.check_parent()?;
        let target = Target::capture(&self.parent, name)?
            .ok_or_else(|| changed("The collision target disappeared; prepare a new Rename."))?;
        let can_replace = self.info.recoverable_replace
            && qualified(&self.parent, self.identity)
            && target.object.kind == Kind::File
            && target.identity != self.identity;
        let outcome = RenameOutcome::Conflict(RenameConflict {
            token: 1,
            target: target.object.clone(),
            can_replace,
        });
        *retained = Some(target);
        Ok(outcome)
    }
    fn run(
        &self,
        name: &NativeText,
        policy: &RenamePolicy,
        context: &CallContext<'_>,
    ) -> RenameOutcome {
        let result = (|| {
            if name.encoding != NameEncoding::UnixBytes && name.encoding != NameEncoding::Utf8 {
                return Err(Failure::new(
                    ErrorCode::Unsupported,
                    "Linux Rename requires raw Unix bytes.",
                ));
            }
            name.validate_rename_basename()
                .map_err(|_| Failure::new(ErrorCode::Invalid, "Invalid Rename basename."))?;
            budget(
                self.parent_path.as_os_str().as_bytes().len(),
                self.name.as_bytes().len(),
                name.bytes.len(),
            )?;
            let destination = CString::new(name.bytes.as_slice())
                .map_err(|_| Failure::new(ErrorCode::Invalid, "NUL in destination name."))?;
            system::cancel(context)?;
            self.revalidate()?;
            if destination == self.name {
                return Ok(RenameOutcome::Renamed {
                    name: name.clone(),
                    same_entry: true,
                    recovery: vec![],
                    durability_confirmed: true,
                });
            }
            let mut retained = self
                .conflict
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let replacement = match policy {
                RenamePolicy::NoReplace => {
                    *retained = None;
                    if Target::capture(&self.parent, &destination)?.is_some() {
                        return self.observe_conflict(&destination, &mut retained);
                    }
                    None
                }
                RenamePolicy::RecoverableCurrentEntry { .. } => {
                    let Some(target) = retained.as_ref() else {
                        return Err(changed("No retained native collision consent."));
                    };
                    if !self.info.recoverable_replace
                        || !qualified(&self.parent, self.identity)
                        || target.object.kind != Kind::File
                        || target.identity == self.identity
                    {
                        return Err(Failure::new(
                            ErrorCode::Unsupported,
                            "This collision cannot be replaced recoverably.",
                        ));
                    }
                    if target.current(&self.parent, &destination).is_err() {
                        return self.observe_conflict(&destination, &mut retained);
                    }
                    Some((target.identity, target.revision))
                }
            };
            trusted_parent(&self.parent)?;
            system::cancel(context)?;
            self.revalidate()?;
            let mut stage = match Stage::create(self, name) {
                Ok(stage) => stage,
                Err(failure) => return Ok(failure.into_outcome()),
            };
            let capture = (|| {
                self.revalidate()?;
                stage.validate(self)?;
                system::cancel(context)?;
                #[cfg(test)]
                crate::rename_test_hooks::fire(
                    crate::rename_test_hooks::Boundary::BeforeCapture,
                    stage.path(),
                );
                sys::rename(&self.parent, &self.name, stage.file(), ITEM, false)
            })();
            if let Err(error) = capture {
                return Ok(stage.without_capture(self, error));
            }
            let pinned = stage.pin_item();
            #[cfg(test)]
            crate::rename_test_hooks::fire(
                crate::rename_test_hooks::Boundary::AfterCapture,
                stage.path(),
            );
            let validated = (|| {
                pinned?;
                stage.validate(self)?;
                if stage.item_identity() != Some(self.identity)
                    || Identity::of(&self.source)? != self.identity
                {
                    return Err(changed(
                        "A substituted source was captured; it must not be adopted.",
                    ));
                }
                self.check_parent()?;
                if replacement.is_some() {
                    retained
                        .as_ref()
                        .ok_or_else(|| changed("Native consent missing."))?
                        .current(&self.parent, &destination)?;
                }
                system::cancel(context)?;
                Ok(())
            })();
            if let Err(error) = validated {
                let target_changed = replacement.is_some()
                    && error.code == ErrorCode::SourceChanged
                    && stage.item_identity() == Some(self.identity)
                    && self.check_parent().is_ok()
                    && retained
                        .as_ref()
                        .is_some_and(|target| target.current(&self.parent, &destination).is_err());
                let restored = stage.restore(self, error);
                if target_changed && matches!(restored, RenameOutcome::Unchanged(_)) {
                    return self.observe_conflict(&destination, &mut retained);
                }
                return Ok(restored);
            }
            #[cfg(test)]
            crate::rename_test_hooks::fire(
                crate::rename_test_hooks::Boundary::BeforePublish,
                stage.path(),
            );
            if let Err(error) = system::cancel(context) {
                return Ok(stage.restore(self, error));
            }
            stage.mark_publication(replacement.map(|(identity, _)| identity));
            match sys::rename(
                stage.file(),
                ITEM,
                &self.parent,
                &destination,
                replacement.is_some(),
            ) {
                Ok(()) => {
                    #[cfg(test)]
                    crate::rename_test_hooks::fire(
                        crate::rename_test_hooks::Boundary::AfterCommit,
                        stage.path(),
                    );
                    Ok(stage.committed(self, &destination, name, replacement))
                }
                Err(error) => {
                    let collision = error.os_code == Some(libc::EEXIST);
                    let restored = stage.restore(self, error);
                    if collision && matches!(restored, RenameOutcome::Unchanged(_)) {
                        self.observe_conflict(&destination, &mut retained)
                    } else {
                        Ok(restored)
                    }
                }
            }
        })();
        result.unwrap_or_else(RenameOutcome::Unchanged)
    }
}
impl NativeRename for Prepared {
    fn prepared(&self) -> PreparedRenameInfo {
        self.info.clone() // cached; no native I/O and no new authority
    }
    fn execute(
        &self,
        name: &NativeText,
        policy: &RenamePolicy,
        context: &CallContext<'_>,
    ) -> RenameOutcome {
        self.run(name, policy, context)
    }
}
