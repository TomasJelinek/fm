//! Blocking, identity-pinned copy capabilities. Call only on backend workers.
//! Source data is never removed. Replacement requires an explicit provider contract.
use crate::{Cancellation, FsError, MutationItem};
use std::{
    ffi::OsStr,
    io::{Read, Seek, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

/// Provider-native identity; callers compare it but must not reconstruct paths from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TransferIdentity {
    pub device: u64,
    pub object: [u8; 16],
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
    /// Optional recoverable Move preparation. Ordinary Copy and virtual providers
    /// gain no source-mutation authority from their existing read capabilities.
    fn prepare_move_source(
        &self,
        path: &Path,
        _cancel: &Cancellation,
    ) -> Result<crate::PreparedTransferMove, FsError> {
        Err(transfer_unsupported(
            path,
            "recoverable Move is unsupported",
        ))
    }
    fn prepare_transfer_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError>;
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError>;
}

/// A regular file or directory opened without following the selected entry's links.
/// Iteration opens at most one child per call. Retain directory sources on the traversal stack.
pub trait TransferSource: Send {
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
    fn next_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<Box<dyn TransferSource>>, FsError>;
}

pub trait TransferDirectory: Send + Sync {
    fn path(&self) -> &Path;
    fn identity(&self) -> TransferIdentity;
    fn capabilities(&self) -> TransferCapabilities {
        TransferCapabilities::default()
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
/// Content-only policy: ACLs, timestamps, alternate streams and extended attributes are not promised.
pub trait StagedTransferFile: Read + Write + Seek + Send {
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
