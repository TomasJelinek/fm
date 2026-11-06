//! Blocking, identity-pinned copy capabilities. Call only on backend workers.
//! Source data is never removed. Existing destinations are never overwritten.
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

pub trait TransferService: Send + Sync {
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
    fn entry(&self) -> &TransferEntry;
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

/// Publication has already happened for both outcomes; never delete the published target
/// in response to a directory-sync error. Source removal remains a separate, absent capability.
#[derive(Clone, Debug)]
pub enum TransferPublication {
    Published(PathBuf),
    PublishedDurabilityUnconfirmed { path: PathBuf, error: FsError },
}

/// A provider-owned exclusive staging inode. Drop cleans up only its own unpublished inode.
/// Content-only policy: ACLs, timestamps, alternate streams and extended attributes are not promised.
pub trait StagedTransferFile: Read + Write + Seek + Send {
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
    fn cleanup(&mut self) -> Result<(), FsError>;
}
