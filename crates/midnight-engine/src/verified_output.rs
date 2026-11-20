//! Verified publication of small generated files: checksum manifests and the empty
//! file of a confirmed Shift+F4 replacement. Backend workers only.
//!
//! The bytes are staged exclusively, synced and read back independently before the
//! name is published. A replacement keeps the displaced file for recovery, and every
//! outcome after the namespace changed is reported, never dropped or undone.
use midnight_core::*;
use midnight_platform_common::transfer::{CopyPhase, VerifiedFileCopy};
use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

// A publication is a few short native namespace calls; they keep the conservative
// exclusive disk admission without classifying the destination.
const ADMISSION: midnight_work_budget::Admission = midnight_work_budget::Admission::Exclusive;

/// A file this module published.
pub(crate) struct Published {
    pub(crate) path: PathBuf,
    pub(crate) verified_bytes: u64,
    /// What the name held before, now kept for recovery: user data that is reported
    /// and never removed here. Empty unless an existing file was replaced.
    pub(crate) recovery: Vec<PathBuf>,
    /// The file is in place, but a later step failed (the folder's durability, the
    /// recovery proof). The message says what happened to the file.
    pub(crate) warning: Option<FsError>,
}

/// Write `bytes` as `path` in `directory`: as a new file that never replaces anything,
/// or, with `target` (the pinned regular file now at that name), as its replacement
/// through the directory's conditional or recoverable replacement.
pub(crate) fn publish(
    directory: &dyn TransferDirectory,
    path: &Path,
    target: Option<&Arc<dyn TransferTarget>>,
    bytes: Vec<u8>,
    cancel: &Cancellation,
) -> Result<Published, FsError> {
    let name = path.file_name().ok_or_else(|| {
        failure(
            path,
            io::ErrorKind::InvalidInput,
            "The output needs a filename.",
        )
    })?;
    let source = Box::new(GeneratedBytes {
        entry: TransferEntry {
            item: MutationItem {
                path: path.to_owned(),
                kind: EntryKind::File,
                metadata: EntryMetadata {
                    size: Some(bytes.len() as u64),
                    ..EntryMetadata::default()
                },
            },
            identity: TransferIdentity {
                device: 0,
                object: [0; 16],
            },
        },
        bytes: bytes.into(),
        offset: 0,
    });
    let stage = with_io(path, cancel, || match target {
        Some(target) => {
            let capabilities = directory.capabilities();
            let policy = if capabilities.conditional_overwrite {
                TransferReplacementPolicy::ConditionalIdentity
            } else if capabilities.recoverable_replacement {
                TransferReplacementPolicy::RecoverableCurrentEntry
            } else {
                return Err(failure(
                    path,
                    io::ErrorKind::Unsupported,
                    "This filesystem does not support safely replacing a file.",
                ));
            };
            directory.stage_file_preserving_target(name, target.clone(), policy, cancel)
        }
        None => directory.stage_file(cancel),
    })?;
    let mut copy = VerifiedFileCopy::new(source, stage)?;
    let mut published = (|| {
        while copy.phase() != CopyPhase::ReadyToPublish {
            copy.step(cancel)?;
        }
        let publication = if let Some(target) = target {
            with_io(path, cancel, || target.revalidate(cancel))?;
            copy.publish_replace(name, cancel)?
        } else {
            copy.publish(name, cancel)?
        };
        outcome(publication, copy.verified_bytes(), target.is_some())
    })();
    if let Err(ref mut primary) = published
        && let Err(cleanup) = copy.cleanup()
    {
        primary
            .message
            .push_str(&format!(" Staging cleanup also failed: {cleanup}"));
    }
    published
}

/// What a publication left in place. Only an outcome that published nothing is an error.
fn outcome(
    publication: TransferPublication,
    verified_bytes: u64,
    replacing: bool,
) -> Result<Published, FsError> {
    let done = if replacing { "replaced" } else { "created" };
    let (path, recovery, warning) = match publication {
        TransferPublication::Published(path) => (path, Vec::new(), None),
        TransferPublication::Replaced { path, recovery } => (path, vec![recovery.path], None),
        TransferPublication::PublishedDurabilityUnconfirmed { path, mut error } => {
            error.message = format!(
                "{} was {done}, but the folder change could not be confirmed as durable: {}",
                path.display(),
                error.message
            );
            (path, Vec::new(), Some(error))
        }
        TransferPublication::RecoveryRequired {
            published: Some(path),
            recovery,
            mut error,
        } => {
            error.message = format!("{} was {done}, but {}", path.display(), error.message);
            let recovery = recovery.into_iter().map(|item| item.path).collect();
            (path, recovery, Some(error))
        }
        TransferPublication::RecoveryRequired {
            published: None,
            recovery,
            mut error,
        } => {
            for item in recovery {
                error
                    .message
                    .push_str(&format!("\nRecovery file: {}", item.path.display()));
            }
            return Err(error);
        }
    };
    Ok(Published {
        path,
        verified_bytes,
        recovery,
        warning,
    })
}

/// Run one short native call of a publication under the shared disk admission.
pub(crate) fn with_io<T>(
    path: &Path,
    cancel: &Cancellation,
    work: impl FnOnce() -> Result<T, FsError>,
) -> Result<T, FsError> {
    let _permit = midnight_work_budget::shared_io()
        .acquire(ADMISSION, || cancel.is_cancelled())
        .ok_or_else(|| {
            failure(
                path,
                io::ErrorKind::Interrupted,
                "Writing the file was cancelled.",
            )
        })?;
    work()
}

fn failure(path: &Path, kind: io::ErrorKind, message: &str) -> FsError {
    FsError::from_io("write file", path, io::Error::new(kind, message))
}

/// The generated content, read from memory by the verified copy.
struct GeneratedBytes {
    entry: TransferEntry,
    bytes: Arc<[u8]>,
    offset: usize,
}
impl TransferSource for GeneratedBytes {
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }
    fn native_identity(&self) -> Option<TransferIdentity> {
        None
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        if cancel.is_cancelled() {
            Err(failure(
                &self.entry.item.path,
                io::ErrorKind::Interrupted,
                "Writing the file was cancelled.",
            ))
        } else {
            Ok(())
        }
    }
    fn read_chunk(&mut self, buffer: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        self.revalidate(cancel)?;
        let count = buffer.len().min(self.bytes.len() - self.offset);
        buffer[..count].copy_from_slice(&self.bytes[self.offset..self.offset + count]);
        self.offset += count;
        Ok(count)
    }
    fn next_child(&mut self, _: &Cancellation) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        Err(failure(
            &self.entry.item.path,
            io::ErrorKind::Unsupported,
            "A generated file is not a directory.",
        ))
    }
}

#[cfg(test)]
#[path = "verified_output_tests.rs"]
mod tests;
