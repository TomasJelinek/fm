use super::*;
use midnight_core::{
    StagedTransferFile, TransferCapabilities, TransferPublication, TransferReplacementPolicy,
    TransferTarget,
};
use std::{
    ffi::{OsStr, OsString},
    io::{Read, Seek, SeekFrom, Write},
    os::windows::{ffi::OsStringExt, io::AsRawHandle},
    path::PathBuf,
};
use windows_sys::Wdk::Storage::FileSystem::{
    FILE_CREATE, FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN,
};

#[path = "transfer_descriptor.rs"]
mod descriptor;
#[path = "transfer_link_stage.rs"]
mod link_stage;
#[path = "transfer_preservation.rs"]
mod preservation;
#[path = "transfer_recovery.rs"]
pub(super) mod recovery;
#[path = "transfer_destination_security.rs"]
mod security;
#[path = "transfer_destination_staging.rs"]
mod staging;

#[derive(Clone, Copy, PartialEq, Eq)]
enum CopyIds {
    Native,
    Fat,
    /// This network directory has a native ID. Its children may not.
    NetworkId,
    Network,
}

impl CopyIds {
    fn network(self) -> bool {
        matches!(self, Self::NetworkId | Self::Network)
    }
    fn stable(self) -> bool {
        matches!(self, Self::Native | Self::NetworkId)
    }
    fn identity(self, file: &File, path: &Path) -> Result<TransferIdentity, FsError> {
        match self {
            Self::Native | Self::NetworkId => identity(file, path),
            Self::Fat => native::legacy_identity(file)
                .map(transfer_identity)
                .map_err(|e| error(path, e)),
            Self::Network => super::network_identity::binding(file, path),
        }
    }
    fn child(self, file: &File, path: &Path) -> Result<(Self, TransferIdentity), FsError> {
        if self == Self::NetworkId {
            match native::identity(file) {
                Ok(id) => Ok((self, transfer_identity(id))),
                Err(e) if native::unsupported(&e) => {
                    Ok((Self::Network, Self::Network.identity(file, path)?))
                }
                Err(e) => Err(error(path, e)),
            }
        } else {
            Ok((self, self.identity(file, path)?))
        }
    }
    fn matches_binding(
        self,
        held: &File,
        visible: &File,
        path: &Path,
        expected: TransferIdentity,
    ) -> Result<bool, FsError> {
        // A redirector may let an ancestor move. Even if another object now
        // occupies its old path, the retained handle must still resolve to the
        // binding we captured.
        Ok(self.identity(visible, path)? == expected
            && (self != Self::Network || self.identity(held, path)? == expected))
    }
}

#[derive(Clone)]
pub(super) struct Directory {
    file: Arc<File>,
    path: PathBuf,
    identity: TransferIdentity,
    /// FAT directory indices and network binding keys only describe a name while a
    /// participating handle denies delete sharing. Neither is a persistent file ID.
    ids: CopyIds,
    /// Every ancestor of the selected remote destination supplied a file ID.
    /// Unknown ancestry cannot authorize merging into another selected tree.
    stable_ancestry: bool,
    /// What planning links reads once per job, shared with the folders below this one.
    links: Arc<link_stage::LinkFacts>,
    /// This folder's private staging folder of the job (`stage_job_file`), shared by the
    /// clones of this folder only; the folders below it share the job's staging folders
    /// whose removal failed.
    staging: Arc<staging::Staging>,
    // Last: the final Directory clone closes its native Arc<File> before certification.
    _namespace_owner: super::namespace::NativeOwner,
}
impl Directory {
    pub(super) fn for_copy(file: File, path: PathBuf) -> Result<Self, FsError> {
        let id = native::identity(&file);
        Self::from_copy_probe(file, path, id, |file, path| {
            if super::network_identity::is_network(file, path)? {
                return Ok(Some(CopyIds::Network));
            }
            let (_, _, filesystem) = super::link_destination::volume_facts(file)
                .map_err(|failure| error(path, failure))?;
            Ok(matches!(filesystem.as_str(), "FAT" | "FAT32" | "exFAT").then_some(CopyIds::Fat))
        })
    }

    fn from_copy_probe(
        file: File,
        path: PathBuf,
        id: io::Result<native::Identity>,
        fallback: impl FnOnce(&File, &Path) -> Result<Option<CopyIds>, FsError>,
    ) -> Result<Self, FsError> {
        match id {
            Ok(id) => {
                let network = super::network_identity::is_network(&file, &path)
                    .unwrap_or_else(|_| crate::storage_kind::network(&path));
                let ids = if network {
                    CopyIds::NetworkId
                } else {
                    CopyIds::Native
                };
                let mut directory = Self::new(file, path, transfer_identity(id), ids)?;
                if network {
                    directory.stable_ancestry = directory.has_identified_ancestors();
                }
                Ok(directory)
            }
            Err(failure)
                if native::unsupported(&failure)
                    && let Some(ids) = fallback(&file, &path)? =>
            {
                // The original open may share deletion. Pin the exact directory against
                // renaming before recording a key for its current binding.
                let held = native::reopen(
                    &file,
                    FILE_READ_ATTRIBUTES | FILE_TRAVERSE | SYNCHRONIZE,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                )
                .map_err(|e| error(&path, e))?;
                let identity = ids.identity(&held, &path)?;
                Self::new(held, path, identity, ids)
            }
            Err(failure) => Err(error(&path, failure)),
        }
    }
    pub(in crate::transfer) fn native_file(&self) -> &File {
        &self.file
    }
    /// A folder a job starts from: link facts are read anew for it.
    pub(super) fn from_file(file: File, path: PathBuf) -> Result<Self, FsError> {
        let id = identity(&file, &path)?;
        Self::new(file, path, id, CopyIds::Native)
    }
    fn new(
        file: File,
        path: PathBuf,
        identity: TransferIdentity,
        ids: CopyIds,
    ) -> Result<Self, FsError> {
        if !file.metadata().map_err(|e| error(&path, e))?.is_dir() {
            return Err(invalid(&path, "destination is not a directory"));
        }
        Ok(Self::from_directory_handle(file, path, identity, ids))
    }
    fn from_directory_handle(
        file: File,
        path: PathBuf,
        identity: TransferIdentity,
        ids: CopyIds,
    ) -> Self {
        let namespace_owner = if ids != CopyIds::Native {
            Default::default()
        } else {
            super::namespace::owner(&[identity])
        };
        Self {
            identity,
            ids,
            stable_ancestry: ids != CopyIds::Network,
            file: Arc::new(file),
            path,
            links: Arc::default(),
            staging: Arc::default(),
            _namespace_owner: namespace_owner,
        }
    }
    fn entry(&self, file: &File, path: &Path) -> Result<TransferEntry, FsError> {
        classified_using(file, path, |file, path| {
            self.ids.child(file, path).map(|(_, id)| id)
        })
        .map(|(entry, _, _)| entry)
    }
    fn change<T>(
        &self,
        cancel: &Cancellation,
        operation: impl FnOnce(&mut super::namespace::Mutation<'_>) -> Result<T, FsError>,
    ) -> Result<T, FsError> {
        super::namespace::change(cancel, |mutation| {
            // A server can supply an ID for this folder and none for a newly
            // created child. Reserve before changing the namespace, while an
            // unknown binding can still veto the operation.
            if self.ids != CopyIds::Native {
                mutation.reserve_opaque(&self.path, cancel)?;
            }
            operation(mutation)
        })
    }
    /// This folder as one of the same job as `parent` (created or merged into below it),
    /// sharing the link facts `parent` already read.
    pub(super) fn below(mut self, parent: &Directory) -> Self {
        self.stable_ancestry &= parent.stable_ancestry;
        self.links = parent.links.clone();
        self.staging = Arc::new(staging::Staging::below(&parent.staging));
        self
    }
}
impl TransferDirectory for Directory {
    fn native_child_namespace_scope(
        &self,
        name: &OsStr,
        kind: EntryKind,
        cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn midnight_core::TransferNamespacePin>>, FsError> {
        self.revalidate(cancel)?;
        if self.ids != CopyIds::Native {
            return Ok(None);
        }
        super::namespace::child_pin(&self.file, &self.path, name, kind, cancel).map(Some)
    }
    fn open_read_child(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.revalidate(cancel)?;
        super::source::Source::open(self.file.clone(), self.path.clone(), name, cancel)
            .map(|source| Box::new(source) as Box<dyn TransferSource>)
    }
    fn path(&self) -> &Path {
        &self.path
    }
    fn identity(&self) -> TransferIdentity {
        self.identity
    }
    fn capabilities(&self) -> TransferCapabilities {
        TransferCapabilities {
            directory_merge: true,
            conditional_overwrite: false,
            recoverable_replacement: recovery::supported(self),
            // A link is published only without replacing anything (transfers.md, "Links
            // in Copy and Move"): an existing name is skipped or renamed.
            link_replacement: false,
        }
    }
    fn creates_no_links(&self, cancel: &Cancellation) -> Result<Option<String>, FsError> {
        self.no_links_reason(cancel)
    }
    fn admit_link(
        &self,
        representation: &midnight_core::TransferLinkRepresentation,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.plan_link_representation(representation, cancel)
            .map(|_| ())
    }
    fn plan_link(
        &self,
        representation: &midnight_core::TransferLinkRepresentation,
        cancel: &Cancellation,
    ) -> Result<midnight_core::TransferLinkPlan, FsError> {
        self.plan_link_representation(representation, cancel)
    }
    fn stage_link(
        &self,
        representation: &midnight_core::TransferLinkRepresentation,
        cancel: &Cancellation,
    ) -> Result<Box<dyn midnight_core::StagedTransferLink>, midnight_core::TransferLinkStageError>
    {
        self.create_link_stage(representation, cancel)
    }
    fn supports_native_move_replacement(&self, source: TransferIdentity) -> bool {
        source.device == self.identity.device && recovery::supported(self)
    }
    fn prepare_target(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn TransferTarget>>, FsError> {
        recovery::prepare_target(self, name, cancel)
    }
    fn prepare_target_with(
        &self,
        name: &OsStr,
        revisions: midnight_core::TransferRevisionPolicy,
        cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn TransferTarget>>, FsError> {
        recovery::prepare_target_with(self, name, revisions, cancel)
    }
    fn open_merge_directory(
        &self,
        name: &OsStr,
        expected: &TransferEntry,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        recovery::open_merge(self, name, expected, cancel)
    }
    fn stage_file_for_target(
        &self,
        name: &OsStr,
        expected: Arc<dyn TransferTarget>,
        policy: TransferReplacementPolicy,
        cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferFile>, FsError> {
        let mut stage = self.stage_file(cancel)?;
        initialize_stage(stage.as_mut(), |stage| {
            stage.prepare_replacement(name, expected, policy, cancel)
        })?;
        Ok(stage)
    }
    fn stage_file_preserving_target(
        &self,
        name: &OsStr,
        expected: Arc<dyn TransferTarget>,
        policy: TransferReplacementPolicy,
        cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferFile>, FsError> {
        let preserved = preservation::Snapshot::capture(self, name, expected.as_ref(), cancel)?;
        let mut stage = self.create_stage(false, Some(preserved), cancel)?;
        initialize_stage(&mut stage, |stage| {
            stage
                .preserved
                .as_ref()
                .ok_or_else(|| invalid(&stage.path, "missing metadata"))?
                .initialize(&stage.file, &stage.path)?;
            expected.revalidate(cancel)?;
            stage.prepare_replacement(name, expected, policy, cancel)
        })?;
        Ok(Box::new(stage))
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        checked(cancel, &self.path)?;
        let visible = native::open_parent(&self.path).map_err(|e| error(&self.path, e))?;
        if !self
            .ids
            .matches_binding(&self.file, &visible, &self.path, self.identity)?
        {
            return Err(invalid(&self.path, "destination directory changed"));
        }
        Ok(())
    }
    fn inspect(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Option<TransferEntry>, FsError> {
        self.revalidate(cancel)?;
        validate_name(name).map_err(|e| error(&self.path, e))?;
        let path = self.path.join(name);
        match native::open_relative(
            &self.file,
            name,
            FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_OPEN,
            0,
        ) {
            Ok(file) => self.entry(&file, &path).map(Some),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(error(&path, e)),
        }
    }
    fn reject_source_ancestor(
        &self,
        source: TransferIdentity,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.revalidate(cancel)?;
        // Resolve the retained handle, then inspect native identities of its actual ancestors.
        // There is no lexical case folding or assumption that drive letters are identities.
        let actual = final_path(&self.file, VOLUME_NAME_DOS).map_err(|e| error(&self.path, e))?;
        for ancestor in actual.ancestors().filter(|p| p.has_root()) {
            checked(cancel, &self.path)?;
            let file = native::open_parent(ancestor).map_err(|e| error(&self.path, e))?;
            if self.ancestor_identity(&file, ancestor)? == source {
                return Err(invalid(
                    &self.path,
                    "destination is inside the source directory",
                ));
            }
        }
        self.revalidate(cancel)
    }
    fn ancestor_identities(
        &self,
        cancel: &Cancellation,
    ) -> Result<Option<Vec<TransferIdentity>>, FsError> {
        self.revalidate(cancel)?;
        // The same walk as `reject_source_ancestor`: the retained handle's actual path,
        // then the native identity of each ancestor, this folder first.
        let actual = final_path(&self.file, VOLUME_NAME_DOS).map_err(|e| error(&self.path, e))?;
        let mut identities = Vec::new();
        for ancestor in actual.ancestors().filter(|p| p.has_root()) {
            checked(cancel, &self.path)?;
            let file = native::open_parent(ancestor).map_err(|e| error(&self.path, e))?;
            identities.push(self.ancestor_identity(&file, ancestor)?);
        }
        self.revalidate(cancel)?;
        Ok(Some(identities))
    }
    fn create_directory(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.create_copy_directory(name, false, cancel)
    }
    fn create_copy_directory(
        &self,
        name: &OsStr,
        unidentified_source: bool,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.revalidate(cancel)?;
        validate_name(name).map_err(|e| error(&self.path, e))?;
        let path = self.path.join(name);
        // Source folders are listed through handles that deny delete sharing. Asking
        // for DELETE on the output makes a later source open of that same directory
        // fail, even if two drive letters, share names or DFS paths alias each other.
        let guard_access = if unidentified_source || self.ids.network() {
            DELETE
        } else {
            0
        };
        self.change(cancel, |mutation| {
            let file = mutation
                .create(
                    &self.file,
                    &self.path,
                    name,
                    super::namespace::Creation::Planned,
                    cancel,
                    || {
                        native::open_relative(
                            &self.file,
                            name,
                            FILE_READ_ATTRIBUTES | FILE_TRAVERSE | SYNCHRONIZE | guard_access,
                            FILE_SHARE_READ | FILE_SHARE_WRITE,
                            FILE_CREATE,
                            FILE_DIRECTORY_FILE,
                        )
                    },
                )
                .map_err(|e| error(&path, e))?;
            let (ids, id) = self.ids.child(&file, &path).map_err(|failure| {
                // The kernel created this directory exclusively. A network
                // folder whose identity/binding cannot be checked must not be
                // left behind, or deleted by resolving its path again.
                if self.ids.network()
                    && let Err(cleanup) = native::classic_dispose(&file)
                {
                    return error(
                        &path,
                        io::Error::new(
                            cleanup.kind(),
                            format!("{failure}; could not remove the new directory: {cleanup}"),
                        ),
                    );
                }
                failure
            })?;
            let child = Self::from_directory_handle(file, path, id, ids).below(self);
            Ok(Arc::new(child) as Arc<dyn TransferDirectory>)
        })
    }
    fn stage_file(&self, cancel: &Cancellation) -> Result<Box<dyn StagedTransferFile>, FsError> {
        self.make_stage(false, cancel)
    }
    fn stage_job_file(
        &self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferFile>, FsError> {
        match staging::stage(self, cancel)? {
            Some(stage) => Ok(Box::new(stage)),
            None => self.stage_file(cancel),
        }
    }
    fn finish_staging(&self) -> Result<(), FsError> {
        staging::finish(self)
    }
    fn stage_file_with_metadata(
        &self,
        policy: midnight_core::TransferMetadataPolicy,
        cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferFile>, FsError> {
        if policy == midnight_core::TransferMetadataPolicy::PreserveTimesAndAttributes {
            // A stage of the job, as `stage_job_file`, whose handle may also set its
            // times and attributes once it is published (`apply_preserved_metadata`).
            return match staging::stage_with(self, FILE_WRITE_ATTRIBUTES, cancel)? {
                Some(stage) => Ok(Box::new(stage)),
                None => self
                    .create_stage_with(false, true, None, cancel, || stage_name(&self.path))
                    .map(|stage| Box::new(stage) as Box<dyn StagedTransferFile>),
            };
        }
        self.make_stage(
            policy == midnight_core::TransferMetadataPolicy::RequireModifiedTime,
            cancel,
        )
    }
    fn apply_preserved_metadata(
        &self,
        metadata: &midnight_core::TransferPreservedMetadata,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        checked(cancel, &self.path)?;
        // The folder's own handle may only read its attributes and traverse it: reopen
        // the retained object (an empty relative name, never the path) to set them.
        let file = native::reopen(
            &self.file,
            FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES | SYNCHRONIZE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        )
        .map_err(|e| error(&self.path, e))?;
        if self.ids.identity(&file, &self.path)? != self.identity {
            return Err(invalid(&self.path, "the copied folder changed"));
        }
        super::preserve::apply(&file, &self.path, metadata)
    }
}
impl Directory {
    fn has_identified_ancestors(&self) -> bool {
        let Ok(actual) = final_path(&self.file, VOLUME_NAME_DOS) else {
            return false;
        };
        actual
            .ancestors()
            .skip(1)
            .filter(|p| p.has_root())
            .all(|p| {
                native::open_parent(p)
                    .and_then(|ancestor| native::identity(&ancestor))
                    .is_ok()
            })
    }

    fn ancestor_identity(&self, file: &File, path: &Path) -> Result<TransferIdentity, FsError> {
        if self.ids.network() && !self.stable_ancestry {
            match native::identity(file) {
                Ok(id) => Ok(transfer_identity(id)),
                Err(e) if native::unsupported(&e) => super::network_identity::binding(file, path),
                Err(e) => Err(error(path, e)),
            }
        } else if self.ids == CopyIds::Fat {
            // A directory mount can lead from an NTFS ancestor to this FAT volume.
            // Compare that ancestor using its native 128-bit identity.
            native::local_identity(file)
                .map(transfer_identity)
                .map_err(|e| error(path, e))
        } else {
            identity(file, path)
        }
    }
    fn make_stage(
        &self,
        metadata: bool,
        cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferFile>, FsError> {
        self.create_stage(metadata, None, cancel)
            .map(|stage| Box::new(stage) as Box<dyn StagedTransferFile>)
    }
    fn create_stage(
        &self,
        metadata: bool,
        preserved: Option<preservation::Snapshot>,
        cancel: &Cancellation,
    ) -> Result<Stage, FsError> {
        self.create_stage_named(metadata, preserved, cancel, || stage_name(&self.path))
    }

    fn create_stage_named(
        &self,
        metadata: bool,
        preserved: Option<preservation::Snapshot>,
        cancel: &Cancellation,
        next_name: impl FnMut() -> Result<OsString, FsError>,
    ) -> Result<Stage, FsError> {
        self.create_stage_with(metadata, metadata, preserved, cancel, next_name)
    }

    /// `attributes`: the stage's handle may also write its times and attributes.
    fn create_stage_with(
        &self,
        metadata: bool,
        attributes: bool,
        preserved: Option<preservation::Snapshot>,
        cancel: &Cancellation,
        mut next_name: impl FnMut() -> Result<OsString, FsError>,
    ) -> Result<Stage, FsError> {
        self.revalidate(cancel)?;
        for _ in 0..16 {
            checked(cancel, &self.path)?;
            let name = next_name()?;
            let path = self.path.join(&name);
            let created = self.change(cancel, |mutation| {
                let file = mutation
                    .create(
                        &self.file,
                        &self.path,
                        &name,
                        super::namespace::Creation::Private(EntryKind::File),
                        cancel,
                        || {
                            if let Some(preserved) = &preserved {
                                recovery::create_private_file_with(
                                    &self.file,
                                    &name,
                                    preserved.creation_attributes(),
                                )
                            } else {
                                native::open_relative(
                                    &self.file,
                                    &name,
                                    FILE_READ_DATA
                                        | FILE_WRITE_DATA
                                        | FILE_READ_ATTRIBUTES
                                        | DELETE
                                        | SYNCHRONIZE
                                        | if attributes { FILE_WRITE_ATTRIBUTES } else { 0 },
                                    FILE_SHARE_READ,
                                    FILE_CREATE,
                                    FILE_NON_DIRECTORY_FILE,
                                )
                            }
                        },
                    )
                    .map_err(|e| error(&path, e))?;
                let namespace_owner = if self.ids != CopyIds::Native {
                    Default::default()
                } else {
                    super::namespace::owner_for(&file, &path)
                };
                Ok((file, namespace_owner))
            });
            match created {
                Ok((file, namespace_owner)) => {
                    return Ok(Stage {
                        file,
                        directory: self.clone(),
                        published: false,
                        published_name: None,
                        cleaned: false,
                        path,
                        replacement: None,
                        metadata,
                        attributes,
                        preserved,
                        _namespace_owner: namespace_owner,
                    });
                }
                Err(e) if e.kind == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(error(
            &self.path,
            io::Error::new(
                io::ErrorKind::AlreadyExists,
                "exclusive staging names collided repeatedly",
            ),
        ))
    }
}

/// A stage's basename: `.me-` and sixteen random hex characters.
fn stage_name(folder: &Path) -> Result<OsString, FsError> {
    let mut random = [0u8; 8];
    getrandom::fill(&mut random).map_err(|e| error(folder, io::Error::other(e.to_string())))?;
    let suffix = random
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(OsString::from(format!(".me-{suffix}")))
}

#[cfg(test)]
#[path = "transfer_stage_tests.rs"]
mod stage_tests;

#[cfg(test)]
#[path = "transfer_destination_staging_tests.rs"]
mod staging_tests;

struct Stage {
    metadata: bool,
    /// The handle may write the stage's times and attributes (a required-metadata stage,
    /// or one of a job that preserves them).
    attributes: bool,
    preserved: Option<preservation::Snapshot>,
    file: File,
    directory: Directory,
    published: bool,
    published_name: Option<OsString>,
    cleaned: bool,
    path: PathBuf,
    replacement: Option<recovery::Replacement>,
    // Last, after the file, parent and replacement guard fields actually drop.
    _namespace_owner: super::namespace::NativeOwner,
}

/// Construction failures still own an unpublished stage. Preserve both the primary
/// error and any exact-path cleanup refusal; Drop is only a final best-effort retry.
fn initialize_stage<T: StagedTransferFile + ?Sized>(
    stage: &mut T,
    initialize: impl FnOnce(&mut T) -> Result<(), FsError>,
) -> Result<(), FsError> {
    initialize(stage).map_err(|failure| {
        if let Err(mut cleanup) = stage.cleanup() {
            // Preserve the native cleanup kind and residual path: an Interrupted
            // setup error must not hide this refusal as ordinary cancellation.
            cleanup
                .message
                .push_str(&format!("; stage initialization failed: {failure}"));
            return cleanup;
        }
        failure
    })
}
impl Read for Stage {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        self.file.read(b)
    }
}
impl Write for Stage {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.file.write(b)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}
impl Seek for Stage {
    fn seek(&mut self, p: SeekFrom) -> io::Result<u64> {
        self.file.seek(p)
    }
}
impl StagedTransferFile for Stage {
    fn set_modified_time(
        &mut self,
        expected: std::time::SystemTime,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        if self.published || self.cleaned || !self.metadata {
            return Err(unsupported(
                &self.path,
                "modified time can only be set on an owned metadata stage before it gets its final name",
            ));
        }
        super::metadata::set(&self.file, &self.path, expected, cancel)
    }
    fn modified_time(&self, cancel: &Cancellation) -> Result<std::time::SystemTime, FsError> {
        if self.cleaned || !self.metadata {
            return Err(unsupported(
                &self.path,
                "required modified time is unsupported by this stage",
            ));
        }
        super::metadata::modified(&self.file, &self.path, cancel)
    }
    fn apply_preserved_metadata(
        &mut self,
        metadata: &midnight_core::TransferPreservedMetadata,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        let path = match &self.published_name {
            Some(name) => self.directory.path.join(name),
            None => self.path.clone(),
        };
        if !self.published || self.cleaned || !self.attributes {
            return Err(unsupported(
                &path,
                "times and attributes are set only on a published copy whose handle may write them",
            ));
        }
        checked(cancel, &path)?;
        // Through the handle that wrote the copy: Windows then keeps these times when it
        // closes, instead of the time of the last write.
        // Only a published copy gets Read-only, so the cleanup of an unpublished stage
        // never meets it.
        super::preserve::apply(&self.file, &path, metadata)
    }
    fn rewind_for_validation(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn midnight_core::TransferReadGuard>, FsError> {
        if self.cleaned {
            return Err(invalid(&self.path, "stage was cleaned up"));
        }
        super::validation::rewind(
            &self.file,
            &self.path,
            cancel,
            self.directory.ids == CopyIds::Native,
        )
    }
    fn revalidate_publication(&self, cancel: &Cancellation) -> Result<TransferIdentity, FsError> {
        let name = self
            .published_name
            .as_ref()
            .ok_or_else(|| invalid(&self.path, "the copy has not been put in place"))?;
        // FAT directory indices and remote name bindings may change on rename.
        // Check the retained stage against its final directory entry.
        let published = self.directory.path.join(name);
        let (_, own) = self.directory.ids.child(&self.file, &published)?;
        let visible = self.directory.inspect(name, cancel)?.ok_or_else(|| {
            invalid(
                &self.path,
                "the copied file disappeared from the destination",
            )
        })?;
        if visible.item.kind != EntryKind::File || visible.identity != own {
            return Err(invalid(
                &self.path,
                "the copied file's name at the destination changed",
            ));
        }
        Ok(own)
    }
    fn prepare_replacement(
        &mut self,
        name: &OsStr,
        expected: Arc<dyn TransferTarget>,
        policy: TransferReplacementPolicy,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.bind_replacement(name, expected, policy, cancel)
    }
    fn publish_replace(
        &mut self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<TransferPublication, FsError> {
        if let Some(snapshot) = &self.preserved {
            snapshot.verify(&self.file, &self.path)?;
        }
        self.replace(name, cancel, || {})
    }
    fn sync(&mut self, cancel: &Cancellation) -> Result<(), FsError> {
        checked(cancel, &self.path)?;
        if let Some(snapshot) = &self.preserved {
            snapshot.apply_attributes(&self.file, &self.path)?;
            snapshot.verify(&self.file, &self.path)?;
        }
        self.file.sync_all().map_err(|e| error(&self.path, e))
    }
    fn len(&self) -> Result<u64, FsError> {
        self.file
            .metadata()
            .map(|m| m.len())
            .map_err(|e| error(&self.path, e))
    }
    fn publish_no_replace(
        &mut self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<TransferPublication, FsError> {
        if self.published || self.cleaned {
            return Err(invalid(&self.path, "stage is no longer available"));
        }
        self.directory.revalidate(cancel)?;
        validate_name(name).map_err(|e| error(&self.path, e))?;
        let path = self.directory.path.join(name);
        checked(cancel, &path)?;
        self.directory.change(cancel, |mutation| {
            mutation
                .rename(&self.file, &self.directory.file, name, cancel)
                .map_err(|e| error(&path, e))
        })?;
        self.published = true;
        self.published_name = Some(name.to_owned());
        // Windows has no general directory-entry flush contract for this API. The
        // staged file was synced before verification; no parent power-loss guarantee is made.
        Ok(TransferPublication::Published(path))
    }
    fn cleanup(&mut self) -> Result<(), FsError> {
        if !self.published && !self.cleaned {
            // Disposition is bound to our retained inode, never to a pathname that
            // another process could replace. Denied delete sharing protects this handle.
            let cancel = Cancellation::default();
            // A stage that keeps a read-only original's attributes carries its read-only
            // attribute: this owned stage is removed whatever that attribute says.
            let read_only = self.preserved.as_ref().is_some_and(|kept| kept.read_only());
            self.directory.change(&cancel, |mutation| {
                mutation
                    .remove(&self.file, &cancel, || {
                        if self.directory.ids != CopyIds::Native {
                            native::classic_dispose(&self.file)
                        } else if read_only {
                            native::remove_confirmed(&self.file)
                        } else {
                            native::remove(&self.file)
                        }
                    })
                    .map_err(|e| error(&self.path, e))
            })?;
            self.cleaned = true;
        }
        Ok(())
    }
}
impl Drop for Stage {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

/// `GetFinalPathNameByHandleW` with the given volume-name form, without a length cap.
pub(super) fn final_path(file: &File, flags: u32) -> io::Result<PathBuf> {
    let mut buffer = vec![0u16; 256];
    loop {
        let capacity = u32::try_from(buffer.len()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "native path length is unrepresentable",
            )
        })?;
        // SAFETY: writable UTF-16 buffer and live handle; length result checked before use.
        let count = unsafe {
            GetFinalPathNameByHandleW(file.as_raw_handle(), buffer.as_mut_ptr(), capacity, flags)
        };
        if count == 0 {
            return Err(io::Error::last_os_error());
        }
        if count >= capacity {
            buffer.resize(count as usize + 1, 0);
            continue;
        }
        buffer.truncate(count as usize);
        return Ok(PathBuf::from(OsString::from_wide(&buffer)));
    }
}
