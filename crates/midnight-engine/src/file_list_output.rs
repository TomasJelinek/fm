//! Verified native publication of generated file lists. Backend workers only.
use midnight_core::*;
use midnight_platform_common::transfer::{CopyPhase, VerifiedFileCopy};
use std::{
    io,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

pub(crate) struct FileListDestinationPin {
    path: PathBuf,
    append: bool,
    target: Arc<dyn TransferTarget>,
    directory: Arc<dyn TransferDirectory>,
}

impl FileListDestinationPin {
    pub(crate) fn entry(&self) -> &TransferEntry {
        self.target.entry()
    }
}

pub(crate) struct FileListSaveReceipt {
    pub path: PathBuf,
    pub verified_bytes: u64,
    pub recovery: Vec<PathBuf>,
    pub warning: Option<FsError>,
}

pub(crate) enum FileListSaveOutcome {
    Conflict(FileListDestinationPin),
    Saved(FileListSaveReceipt),
}

pub(crate) fn save_file_list(
    mut source: Box<dyn TransferSource>,
    destination: &Path,
    append: bool,
    expected: Option<FileListDestinationPin>,
    service: &dyn TransferService,
    cancel: &Cancellation,
) -> Result<FileListSaveOutcome, FsError> {
    if !destination.is_absolute()
        || destination.as_os_str().as_encoded_bytes().contains(&0)
        || destination
            .components()
            .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
    {
        return Err(failure(
            destination,
            io::ErrorKind::InvalidInput,
            "Choose an absolute output path without parent steps or NUL.",
        ));
    }
    let (Some(parent), Some(name)) = (destination.parent(), destination.file_name()) else {
        return Err(failure(
            destination,
            io::ErrorKind::InvalidInput,
            "The output path must include a filename.",
        ));
    };
    source_length(source.as_ref())?;
    let (directory, target) = match expected {
        Some(pin) => {
            if pin.path != destination || pin.append != append {
                return Err(failure(
                    destination,
                    io::ErrorKind::InvalidInput,
                    "The confirmed output path or append mode changed; choose it again.",
                ));
            }
            with_io(destination, cancel, || {
                pin.directory.revalidate(cancel)?;
                pin.target.revalidate(cancel)
            })?;
            validate_target(source.as_ref(), pin.target.as_ref(), destination)?;
            (pin.directory, Some(pin.target))
        }
        None => {
            let directory = with_io(parent, cancel, || {
                service.prepare_transfer_destination(parent, cancel)
            })?;
            if let Some(target) = with_io(destination, cancel, || {
                directory.prepare_target(name, cancel)
            })? {
                validate_target(source.as_ref(), target.as_ref(), destination)?;
                return Ok(FileListSaveOutcome::Conflict(FileListDestinationPin {
                    path: destination.to_owned(),
                    append,
                    directory,
                    target,
                }));
            }
            (directory, None)
        }
    };
    if append && let Some(target) = &target {
        let prefix = with_io(destination, cancel, || {
            target.revalidate(cancel)?;
            let prefix = directory.open_read_child(name, cancel)?;
            if prefix.native_identity() != Some(target.entry().identity) {
                return Err(failure(
                    destination,
                    io::ErrorKind::InvalidData,
                    "The existing output changed before append.",
                ));
            }
            target.revalidate(cancel)?;
            Ok(prefix)
        })?;
        source = Box::new(AppendedSource::new(prefix, source, target.clone())?);
    }
    let stage = with_io(destination, cancel, || match &target {
        Some(target) => {
            let capabilities = directory.capabilities();
            let policy = if capabilities.conditional_overwrite {
                TransferReplacementPolicy::ConditionalIdentity
            } else if capabilities.recoverable_replacement {
                TransferReplacementPolicy::RecoverableCurrentEntry
            } else {
                return Err(failure(
                    destination,
                    io::ErrorKind::Unsupported,
                    "This filesystem does not support safe output replacement or append.",
                ));
            };
            directory.stage_file_preserving_target(name, target.clone(), policy, cancel)
        }
        None => directory.stage_file(cancel),
    })?;
    let mut copy = VerifiedFileCopy::new(source, stage)?;
    let mut result = (|| {
        while copy.phase() != CopyPhase::ReadyToPublish {
            copy.step(cancel)?;
        }
        let publication = if let Some(target) = &target {
            with_io(destination, cancel, || target.revalidate(cancel))?;
            copy.publish_replace(name, cancel)?
        } else {
            copy.publish(name, cancel)?
        };
        receipt(publication, copy.verified_bytes()).map(FileListSaveOutcome::Saved)
    })();
    if let Err(primary) = &mut result
        && let Err(cleanup) = copy.cleanup()
    {
        primary
            .message
            .push_str(&format!(" Staging cleanup also failed: {cleanup}"));
    }
    result
}

fn validate_target(
    source: &dyn TransferSource,
    target: &dyn TransferTarget,
    destination: &Path,
) -> Result<(), FsError> {
    if target.entry().item.kind != EntryKind::File {
        return Err(failure(
            destination,
            io::ErrorKind::InvalidInput,
            "The output already names a folder, link or special file.",
        ));
    }
    if source.native_identity() == Some(target.entry().identity) {
        return Err(failure(
            destination,
            io::ErrorKind::InvalidInput,
            "The output aliases its generated source.",
        ));
    }
    Ok(())
}

fn receipt(
    publication: TransferPublication,
    verified_bytes: u64,
) -> Result<FileListSaveReceipt, FsError> {
    let (path, recovery, warning) = match publication {
        TransferPublication::Published(path) => (path, Vec::new(), None),
        TransferPublication::PublishedDurabilityUnconfirmed { path, error } => {
            (path, Vec::new(), Some(error))
        }
        TransferPublication::Replaced { path, recovery } => (path, vec![recovery.path], None),
        TransferPublication::RecoveryRequired {
            published,
            recovery,
            mut error,
        } => {
            let recovery: Vec<_> = recovery.into_iter().map(|item| item.path).collect();
            if let Some(path) = published {
                (path, recovery, Some(error))
            } else {
                for path in recovery {
                    error
                        .message
                        .push_str(&format!("\nRecovery file: {}", path.display()));
                }
                return Err(error);
            }
        }
    };
    Ok(FileListSaveReceipt {
        path,
        verified_bytes,
        recovery,
        warning,
    })
}

/// Append is an independently verified replacement containing old bytes followed
/// by new bytes. A failed/cancelled export never writes into the existing file.
struct AppendedSource {
    entry: TransferEntry,
    prefix: Option<Box<dyn TransferSource>>,
    target: Arc<dyn TransferTarget>,
    suffix: Box<dyn TransferSource>,
    prefix_length: u64,
    prefix_bytes: u64,
}

impl AppendedSource {
    fn new(
        prefix: Box<dyn TransferSource>,
        suffix: Box<dyn TransferSource>,
        target: Arc<dyn TransferTarget>,
    ) -> Result<Self, FsError> {
        let prefix_length = source_length(prefix.as_ref())?;
        let size = prefix_length
            .checked_add(source_length(suffix.as_ref())?)
            .ok_or_else(|| {
                failure(
                    &suffix.entry().item.path,
                    io::ErrorKind::InvalidData,
                    "The combined output length exceeds the filesystem length representation.",
                )
            })?;
        let mut entry = suffix.entry().clone();
        entry.item.metadata = EntryMetadata {
            size: Some(size),
            ..Default::default()
        };
        Ok(Self {
            entry,
            prefix: Some(prefix),
            target,
            suffix,
            prefix_length,
            prefix_bytes: 0,
        })
    }
}

impl TransferSource for AppendedSource {
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }

    fn native_identity(&self) -> Option<TransferIdentity> {
        None
    }

    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        if let Some(prefix) = &self.prefix {
            prefix.revalidate(cancel)?;
        }
        self.target.revalidate(cancel)?;
        self.suffix.revalidate(cancel)
    }

    fn read_chunk(&mut self, buffer: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        if let Some(prefix) = &mut self.prefix {
            let count = prefix.read_chunk(buffer, cancel)?;
            self.prefix_bytes = self.prefix_bytes.checked_add(count as u64).ok_or_else(|| {
                failure(
                    &self.entry.item.path,
                    io::ErrorKind::InvalidData,
                    "Append byte count overflow.",
                )
            })?;
            if count > buffer.len()
                || self.prefix_bytes > self.prefix_length
                || (count == 0 && self.prefix_bytes != self.prefix_length)
            {
                return Err(failure(
                    &prefix.entry().item.path,
                    io::ErrorKind::InvalidData,
                    "The existing output length changed during append.",
                ));
            }
            if count != 0 {
                return Ok(count);
            }
            prefix.revalidate(cancel)?;
            self.target.revalidate(cancel)?;
            // Release the read handle before publication: native readers may deny
            // rename sharing. The original target pin still validates every later
            // verification/publication boundary against its captured revision.
            self.prefix = None;
        }
        self.suffix.read_chunk(buffer, cancel)
    }

    fn next_child(&mut self, _: &Cancellation) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        Err(failure(
            &self.entry.item.path,
            io::ErrorKind::InvalidInput,
            "A generated file list is not a directory.",
        ))
    }
}

fn source_length(source: &dyn TransferSource) -> Result<u64, FsError> {
    (source.entry().item.kind == EntryKind::File)
        .then_some(source.entry().item.metadata.size)
        .flatten()
        .ok_or_else(|| {
            failure(
                &source.entry().item.path,
                io::ErrorKind::InvalidInput,
                "A generated regular file with known length is required.",
            )
        })
}

fn with_io<T>(
    path: &Path,
    cancel: &Cancellation,
    work: impl FnOnce() -> Result<T, FsError>,
) -> Result<T, FsError> {
    let _permit = midnight_work_budget::shared_io()
        .acquire(4, || cancel.is_cancelled())
        .ok_or_else(|| {
            failure(
                path,
                io::ErrorKind::Interrupted,
                "File-list output cancelled.",
            )
        })?;
    work()
}

fn failure(path: &Path, kind: io::ErrorKind, message: &str) -> FsError {
    FsError::from_io("save file list", path, io::Error::new(kind, message))
}

#[cfg(test)]
#[path = "file_list_output_tests.rs"]
mod tests;
