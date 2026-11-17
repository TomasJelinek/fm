//! Blocking, identity-pinned copy capabilities. Call only on backend workers.
//! Ordinary Copy never removes sources; explicit Move finalization is separate.
//! Replacement requires an explicit provider contract.
#[path = "transfer_links.rs"]
mod links;
pub use links::*;

use crate::{Cancellation, FsError, MutationItem};
use std::{
    ffi::OsStr,
    io::{Read, Seek, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

/// Provider-native identity; callers compare it but must not reconstruct paths from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TransferIdentity {
    pub device: u64,
    pub object: [u8; 16],
}

/// Host-assigned access for an entire native namespace pin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferNamespaceAccess {
    Read,
    Write,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferNamespaceExtent {
    Entry,
    Subtree,
}

/// Pooled native parent authority metadata, shared by sibling root claims.
#[derive(Clone, Debug)]
pub struct TransferNamespaceParent {
    pub identity: TransferIdentity,
    /// Qualifies actual native case/alias lookup rules; never display normalization.
    pub lookup_domain: Arc<[u8]>,
    /// Actual native parent first, then its ancestors through the native root.
    pub ancestors: Arc<[TransferIdentity]>,
}

#[derive(Clone, Debug)]
pub struct TransferNamespaceScope {
    /// A native filesystem root uses its own identity as parent, an empty lookup key,
    /// and an ancestor list starting at that root. No display path becomes authority.
    pub parent: Arc<TransferNamespaceParent>,
    /// Comparable only within the same validated parent lookup domain.
    pub lookup_key: Arc<[u8]>,
    /// Absent only for a still-absent prospective slot, including a future subtree.
    pub object: Option<TransferIdentity>,
    pub extent: TransferNamespaceExtent,
}

#[derive(Clone, Debug)]
pub enum TransferNamespaceProbe {
    /// Complete nonempty ordinary-native footprint. Shared ancestors alone do not intersect.
    /// An empty footprint is reserved for a certified removed, registered private alias;
    /// it must never retire an ordinary selected source or destination root.
    Known(Arc<[TransferNamespaceScope]>),
    /// Valid authority whose geometry is unqualified; requires global transfer exclusion.
    Unknown,
}

/// Metadata-only retained original bindings, independently owned beyond source traversal.
/// Pool parent/ancestor handles; probing selected sibling leaves must not retain a handle per leaf.
/// Refresh on a backend worker under lifetime I/O authority, outside registry locks.
/// Never adopt unrelated replacements. Owned name transitions must update their witness first.
pub trait TransferNamespacePin: Send + Sync {
    fn refresh(&self, cancel: &Cancellation) -> Result<TransferNamespaceProbe, FsError>;
}

#[derive(Clone, Debug)]
pub struct TransferEntry {
    pub item: MutationItem,
    pub identity: TransferIdentity,
}

/// Replacement policies are separate capabilities; a provider must never weaken one into another.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferReplacementPolicy {
    ConditionalIdentity,
    /// Replace the current name while retaining every displaced object for recovery.
    /// A provider may have a declared namespace gap; this is not identity compare-and-swap.
    RecoverableCurrentEntry,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TransferCapabilities {
    pub directory_merge: bool,
    pub conditional_overwrite: bool,
    pub recoverable_replacement: bool,
}

/// A pinned native object and its provider-owned revision, captured for one conflict.
pub trait TransferTarget: Send + Sync {
    fn entry(&self) -> &TransferEntry;
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError>;
}

pub trait TransferService: Send + Sync {
    /// Metadata-only advisory native device identity for automatic drag action.
    /// Unknown is safe and means Copy. Never read contents, mutate the namespace,
    /// or acquire source deletion authority. Transfer preparation revalidates pins.
    fn native_drop_device(
        &self,
        _path: &Path,
        _cancel: &Cancellation,
    ) -> Result<Option<u64>, FsError> {
        Ok(None)
    }
    /// Open an existing checksum base without following any path component's links.
    /// This grants no implicit filesystem writes; callers use only open_read_child.
    fn prepare_checksum_directory(
        &self,
        path: &Path,
        _cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        Err(transfer_unsupported(
            path,
            "safe checksum directory reads are unsupported",
        ))
    }
    /// A native destination whose retained stages support later publication proofs.
    fn prepare_move_destination(
        &self,
        path: &Path,
        _cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        Err(transfer_unsupported(
            path,
            "verified Move destination is unsupported",
        ))
    }
    /// Optional ordinary Move preparation. Ordinary Copy and virtual providers
    /// gain no source-mutation authority from their existing read capabilities.
    fn prepare_move_source(
        &self,
        path: &Path,
        _cancel: &Cancellation,
    ) -> Result<crate::PreparedTransferMove, FsError> {
        Err(transfer_unsupported(path, "verified Move is unsupported"))
    }
    fn prepare_transfer_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError>;
    /// Optional explicit link-preserving Copy capture. Reject keeps legacy source
    /// preparation unchanged. Preserve requires a positive no-follow native
    /// capability; never fall back to a readable referent or Move preparation.
    /// Retain this policy in directory sources so next_child/open_child capture
    /// link objects recursively. Capture original raw binding, identity and
    /// revision/representation; retries keep these pins rather than recapturing.
    fn prepare_transfer_source_with_links(
        &self,
        path: &Path,
        links: CopyLinkPolicy,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        match links {
            CopyLinkPolicy::Reject => self.prepare_transfer_source(path, cancel),
            CopyLinkPolicy::Preserve => Err(transfer_unsupported(
                path,
                "link-preserving source preparation is unsupported",
            )),
        }
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError>;
}

/// Audited host composition for the concrete service and every native capability
/// it creates: sources, directories, targets, stages, read guards, metadata and
/// namespace pins, Move captures/finalizers/cleanup tickets, and internal leases.
/// Releasing a reference is synchronous; if it acquires last-owner responsibility,
/// actual native destruction completes before return. No deferred cleanup tail
/// may outlive that return. Shared references release only their own ownership.
/// Caller-supplied captured providers and independently composed owners are not
/// covered. There is no blanket implementation and this is not a plugin C ABI.
pub trait AuditedSynchronousTransferService: TransferService {}

pub(crate) fn unconfirmed_transfer_release(operation: &'static str) -> FsError {
    FsError::from_io(
        operation,
        Path::new(""),
        std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Captured provider retirement is unconfirmed; transfer authority remains retained.",
        ),
    )
}

/// A no-follow regular file, ordinary directory or explicitly preserved link.
/// Iteration opens at most one child per call. Retain directory sources on the traversal stack;
/// preserved links are leaves and never grant referent reads or traversal.
pub trait TransferSource: Send {
    /// Optional exact native root binding/ancestry. None is conservative unknown authority,
    /// not a claim that this source is independent. Construction and refresh are worker-only.
    fn native_namespace_scope(
        &self,
        _cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn TransferNamespacePin>>, FsError> {
        Ok(None)
    }
    /// Independently read the retained native link's exact bounded representation
    /// without opening its target. Success must revalidate the ORIGINAL retained
    /// parent/ancestor bindings and raw source name against the captured object,
    /// not merely certify that a retained descriptor/handle still refers to it.
    /// Compare its current revision/payload against the original capture: an
    /// in-place change must fail even if native identity is unchanged. Never
    /// adopt a new binding, revision or payload, or return cached creation bytes.
    /// The caller repeats this query immediately before publication. Declared
    /// native kind/format, not the display EntryKind, establishes link authority.
    fn link_representation(
        &self,
        _cancel: &Cancellation,
    ) -> Result<TransferLinkRepresentation, FsError> {
        Err(transfer_unsupported(
            &self.entry().item.path,
            "native link representation is unsupported",
        ))
    }
    /// Metadata-only independent traversal of this pinned source. Never consumes its copy
    /// iterator, reads file contents, follows links or grants mutation authority.
    fn discover(
        &self,
        cancel: &Cancellation,
        visit: &mut dyn FnMut(&MutationItem),
    ) -> Result<(), FsError> {
        self.revalidate(cancel)?;
        if self.entry().item.kind == crate::EntryKind::Directory {
            return Err(transfer_unsupported(
                &self.entry().item.path,
                "recursive discovery is unavailable",
            ));
        }
        visit(&self.entry().item);
        Ok(())
    }
    /// Seek the retained regular file during a guarded validation pass, including
    /// after source capture changed its name. Return its current exact length.
    /// Never reopen the original pathname; the caller retains/rechecks the guard
    /// from `rewind_for_validation` around all sampled reads.
    fn seek_for_validation(
        &mut self,
        _offset: u64,
        _cancel: &Cancellation,
    ) -> Result<u64, FsError> {
        Err(transfer_unsupported(
            &self.entry().item.path,
            "retained source range validation is unsupported",
        ))
    }
    /// Requested tree root only; descendant wrappers must return None.
    fn tree_copy_occurrence(&self) -> Option<crate::TreeCopyOccurrence> {
        None
    }
    /// Open one ordinary child relative to this pinned directory, without following links.
    fn open_child(
        &self,
        _name: &OsStr,
        _cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        Err(transfer_unsupported(
            &self.entry().item.path,
            "named child reads are unsupported",
        ))
    }
    /// Optional virtual identity for completion feedback; never a native pathname.
    fn virtual_identity(&self) -> Option<crate::VirtualEntryId> {
        None
    }
    /// Enumerate one no-follow child with explicit native Move authority. Only a
    /// source prepared for Move may implement this; ordinary Copy stays read-only.
    fn next_move_child(
        &mut self,
        _cancel: &Cancellation,
    ) -> Result<Option<crate::PreparedTransferMove>, FsError> {
        Err(transfer_unsupported(
            &self.entry().item.path,
            "recursive Move is unsupported",
        ))
    }
    fn prepare_modified_time(
        &self,
        _cancel: &Cancellation,
    ) -> Result<Box<dyn crate::TransferModifiedTimePin>, FsError> {
        Err(transfer_unsupported(
            &self.entry().item.path,
            "required modified time is unsupported",
        ))
    }
    /// Rewind the retained file and guard its current revision. This must continue
    /// working after recoverable source capture changes its original pathname.
    fn rewind_for_validation(
        &mut self,
        _cancel: &Cancellation,
    ) -> Result<Box<dyn crate::TransferReadGuard>, FsError> {
        Err(transfer_unsupported(
            &self.entry().item.path,
            "retained source validation is unsupported",
        ))
    }
    fn entry(&self) -> &TransferEntry;
    /// Native sources return an OS identity for ancestry checks. Virtual providers
    /// return `None`; their member identities must never be compared to native inodes.
    fn native_identity(&self) -> Option<TransferIdentity> {
        Some(self.entry().identity)
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError>;
    fn read_chunk(&mut self, buffer: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError>;
    /// Whether a failed read can be repositioned on this exact retained source.
    /// Forward-only/virtual streams must leave this disabled unless they implement
    /// the full identity and original-revision contract of `retry_read_from`.
    fn supports_read_retry(&self) -> bool {
        false
    }
    /// Revalidate the original source identity/revision and seek its retained file
    /// to the last committed byte. Never reopen/adopt the current source pathname.
    /// An error may move the cursor; callers must repeat this operation before reading.
    fn retry_read_from(&mut self, _offset: u64, _cancel: &Cancellation) -> Result<(), FsError> {
        Err(transfer_unsupported(
            &self.entry().item.path,
            "retained source read retry is unsupported",
        ))
    }
    fn next_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<Box<dyn TransferSource>>, FsError>;
}

pub trait TransferDirectory: Send + Sync {
    /// Exact prospective native child slot and optional future subtree, under this pinned
    /// parent. Qualify native lookup aliases/case rules; None requires conservative exclusion.
    /// This metadata capability never creates, stages, merges or grants mutation authority.
    fn native_child_namespace_scope(
        &self,
        _name: &OsStr,
        _kind: crate::EntryKind,
        _cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn TransferNamespacePin>>, FsError> {
        Ok(None)
    }
    /// Read-only positive admission for this exact native kind/format/flags and
    /// payload on the retained destination. Reject unsupported formats, local
    /// junction forms, permissions or filesystem features explicitly; never
    /// open a target. Admission does not grant replacement or Move authority.
    fn admit_link(
        &self,
        _representation: &TransferLinkRepresentation,
        _cancel: &Cancellation,
    ) -> Result<(), FsError> {
        Err(transfer_unsupported(
            self.path(),
            "link staging is unsupported",
        ))
    }
    /// Exclusively create an owned native link stage after positive admission;
    /// never normalize/rebase its target or create/open referent content. Validate
    /// format lengths before allocation. After partial creation, failure MUST
    /// return the owned partial stage for explicit worker cleanup; it grants no
    /// publication/resume authority. No-stage failure certifies no owned residue.
    fn stage_link(
        &self,
        _representation: &TransferLinkRepresentation,
        _cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferLink>, TransferLinkStageError> {
        Err(TransferLinkStageError {
            error: transfer_unsupported(self.path(), "link staging is unsupported"),
            stage: None,
        })
    }
    /// Open one existing ordinary child through this pinned native directory.
    fn open_read_child(
        &self,
        _name: &OsStr,
        _cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        Err(transfer_unsupported(
            self.path(),
            "named child reads are unsupported",
        ))
    }
    /// Optional owned-stage authority. ContentOnly callers keep using stage_file.
    fn stage_file_with_metadata(
        &self,
        _policy: crate::TransferMetadataPolicy,
        _cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferFile>, FsError> {
        Err(transfer_unsupported(
            self.path(),
            "required modified time is unsupported",
        ))
    }
    fn path(&self) -> &Path;
    fn identity(&self) -> TransferIdentity;
    fn capabilities(&self) -> TransferCapabilities {
        TransferCapabilities::default()
    }
    /// Whether this directory can retain a displaced ordinary file during a native
    /// move from this source device. This does not qualify copy-based replacement
    /// or source deletion. Eligibility is only a hint; the actual pinned rename
    /// must still validate the volume, target identity and explicit policy.
    fn supports_native_move_replacement(&self, _source: TransferIdentity) -> bool {
        false
    }
    fn prepare_target(
        &self,
        _name: &OsStr,
        _cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn TransferTarget>>, FsError> {
        Err(transfer_unsupported(
            self.path(),
            "pinned transfer targets are unsupported",
        ))
    }
    /// Open only an ordinary directory with the consented identity. The caller keeps its
    /// target pin alive and revalidates its revision immediately before this call.
    fn open_merge_directory(
        &self,
        _name: &OsStr,
        _expected: &TransferEntry,
        _cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        Err(transfer_unsupported(
            self.path(),
            "directory merging is unsupported",
        ))
    }
    fn stage_file_for_target(
        &self,
        _name: &OsStr,
        _expected: Arc<dyn TransferTarget>,
        _policy: TransferReplacementPolicy,
        _cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferFile>, FsError> {
        Err(transfer_unsupported(
            self.path(),
            "recoverable replacement is unsupported",
        ))
    }
    /// Stage a replacement preserving the consented target's access controls and supported
    /// native metadata. Before any content is written, the new object must be no more
    /// accessible than the original. Unsupported security, streams or extended metadata
    /// must fail explicitly, never silently fall back to an ordinary content-only stage.
    /// The target's identity and revision remain subject to the usual replacement policy.
    fn stage_file_preserving_target(
        &self,
        _name: &OsStr,
        _expected: Arc<dyn TransferTarget>,
        _policy: TransferReplacementPolicy,
        _cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferFile>, FsError> {
        Err(transfer_unsupported(
            self.path(),
            "preserving existing output security and metadata is unsupported",
        ))
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError>;
    fn inspect(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Option<TransferEntry>, FsError>;
    /// Fail when this destination is the source directory or below it (native ancestry).
    fn reject_source_ancestor(
        &self,
        source: TransferIdentity,
        cancel: &Cancellation,
    ) -> Result<(), FsError>;
    /// Exclusive creation. Existing directories are conflicts; no implicit tree merging.
    fn create_directory(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError>;
    fn stage_file(&self, cancel: &Cancellation) -> Result<Box<dyn StagedTransferFile>, FsError>;
}

/// Actual publication and retained-recovery facts. Never delete a published target or
/// retained recovery data in response to an error. Source removal is a separate capability.
#[derive(Clone, Debug)]
pub enum TransferPublication {
    Published(PathBuf),
    PublishedDurabilityUnconfirmed {
        path: PathBuf,
        error: FsError,
    },
    Replaced {
        path: PathBuf,
        recovery: TransferRecovery,
    },
    /// A native namespace mutation occurred. Retained recovery entries are user data:
    /// never remove them or a published destination in response to this outcome.
    /// This outcome cannot authorize source deletion by a verified Move.
    RecoveryRequired {
        published: Option<PathBuf>,
        recovery: Vec<TransferRecovery>,
        error: FsError,
    },
}

#[derive(Clone, Debug)]
pub struct TransferRecovery {
    pub path: PathBuf,
    /// None only when native metadata failed after a namespace mutation. The known
    /// recovery path is still retained and must be shown; never invent an identity.
    pub identity: Option<TransferIdentity>,
}

/// A provider-owned exclusive staging inode. Drop cleans up only its own unpublished inode.
/// Default stages are content-only; required modified time is a separate optional
/// authority. Existing-output security and native metadata require the separate
/// fail-closed `TransferDirectory::stage_file_preserving_target` capability.
pub trait StagedTransferFile: Read + Write + Seek + Send {
    /// Change only modified time on the owned unpublished inode, never data or a
    /// published target. The caller separately syncs and reads back the exact value.
    fn set_modified_time(
        &mut self,
        _expected: SystemTime,
        _cancel: &Cancellation,
    ) -> Result<(), FsError> {
        Err(transfer_unsupported(
            Path::new(""),
            "required modified time is unsupported",
        ))
    }
    fn modified_time(&self, _cancel: &Cancellation) -> Result<SystemTime, FsError> {
        Err(transfer_unsupported(
            Path::new(""),
            "required modified time is unsupported",
        ))
    }
    fn rewind_for_validation(
        &mut self,
        _cancel: &Cancellation,
    ) -> Result<Box<dyn crate::TransferReadGuard>, FsError> {
        Err(transfer_unsupported(
            Path::new(""),
            "retained target validation is unsupported",
        ))
    }
    /// Revalidate the final name relative to the held directory against this exact
    /// stage inode, never adopting a concurrent replacement's identity.
    fn revalidate_publication(&self, _cancel: &Cancellation) -> Result<TransferIdentity, FsError> {
        Err(transfer_unsupported(
            Path::new(""),
            "publication validation is unsupported",
        ))
    }
    fn sync(&mut self, cancel: &Cancellation) -> Result<(), FsError>;
    fn len(&self) -> Result<u64, FsError>;
    fn is_empty(&self) -> Result<bool, FsError> {
        self.len().map(|length| length == 0)
    }
    /// Publish exclusively. Err means the final name was not changed by this call;
    /// any successful mutation with failed durability must return a structured outcome.
    fn publish_no_replace(
        &mut self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<TransferPublication, FsError>;
    /// Bind or rebind an unpublished stage after a versioned conflict decision, without
    /// changing the namespace. This also supports a target appearing after verification.
    fn prepare_replacement(
        &mut self,
        _name: &OsStr,
        _expected: Arc<dyn TransferTarget>,
        _policy: TransferReplacementPolicy,
        _cancel: &Cancellation,
    ) -> Result<(), FsError> {
        Err(transfer_unsupported(
            Path::new(""),
            "replacement preparation is unsupported",
        ))
    }
    /// Commit a stage bound to an explicitly selected replacement policy and target pin.
    /// Err means no final destination or user object was moved by this call; an empty
    /// private recovery directory may remain. Partial commits return RecoveryRequired.
    fn publish_replace(
        &mut self,
        _name: &OsStr,
        _cancel: &Cancellation,
    ) -> Result<TransferPublication, FsError> {
        Err(transfer_unsupported(
            Path::new(""),
            "replacement publication is unsupported",
        ))
    }
    fn cleanup(&mut self) -> Result<(), FsError>;
}

fn transfer_unsupported(path: &Path, message: &str) -> FsError {
    FsError::from_io(
        "transfer capability",
        path,
        std::io::Error::new(std::io::ErrorKind::Unsupported, message),
    )
}

#[cfg(test)]
mod namespace_defaults_tests {
    use super::*;

    struct Legacy;
    impl TransferSource for Legacy {
        fn entry(&self) -> &TransferEntry {
            panic!("namespace opt-out must not inspect entries")
        }
        fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
            panic!("namespace opt-out must not perform native validation")
        }
        fn read_chunk(&mut self, _: &mut [u8], _: &Cancellation) -> Result<usize, FsError> {
            panic!("namespace opt-out must not read content")
        }
        fn next_child(
            &mut self,
            _: &Cancellation,
        ) -> Result<Option<Box<dyn TransferSource>>, FsError> {
            panic!("namespace opt-out must not enumerate")
        }
    }
    impl TransferDirectory for Legacy {
        fn path(&self) -> &Path {
            panic!("namespace opt-out must not interpret paths")
        }
        fn identity(&self) -> TransferIdentity {
            panic!("namespace opt-out has no native identity")
        }
        fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
            panic!("unexpected I/O")
        }
        fn inspect(&self, _: &OsStr, _: &Cancellation) -> Result<Option<TransferEntry>, FsError> {
            panic!("namespace opt-out must not probe children")
        }
        fn reject_source_ancestor(
            &self,
            _: TransferIdentity,
            _: &Cancellation,
        ) -> Result<(), FsError> {
            panic!("namespace opt-out must not walk ancestors")
        }
        fn create_directory(
            &self,
            _: &OsStr,
            _: &Cancellation,
        ) -> Result<Arc<dyn TransferDirectory>, FsError> {
            panic!("namespace opt-out must not create objects")
        }
        fn stage_file(&self, _: &Cancellation) -> Result<Box<dyn StagedTransferFile>, FsError> {
            panic!("namespace opt-out must not create stages")
        }
    }

    #[test]
    fn legacy_namespace_capabilities_remain_explicit_opt_out_without_io() {
        let cancel = Cancellation::default();
        assert!(Legacy.native_namespace_scope(&cancel).unwrap().is_none());
        assert!(
            Legacy
                .native_child_namespace_scope(OsStr::new("child"), crate::EntryKind::File, &cancel)
                .unwrap()
                .is_none()
        );
        cancel.cancel();
        assert!(Legacy.native_namespace_scope(&cancel).unwrap().is_none());
        assert!(
            Legacy
                .native_child_namespace_scope(OsStr::new("child"), crate::EntryKind::File, &cancel)
                .unwrap()
                .is_none()
        );
    }
}
