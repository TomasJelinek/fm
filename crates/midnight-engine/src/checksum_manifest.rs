//! GNU binary checksum export. Only explicit backend callers may encode/save.
use super::worker::{failure, with_io};
use midnight_core::*;
use midnight_platform_common::transfer::{CopyPhase, VerifiedFileCopy};
use std::{
    collections::HashSet,
    io,
    path::{Component, Path},
    sync::{Arc, OnceLock},
};

const MAX_MANIFEST_BYTES: usize = 16 * 1024 * 1024;

/// Save through exclusive native staging, sync and independent destination SHA-256 readback.
/// Existing destinations fail explicitly (AlreadyExists); this first backend never overwrites.
pub fn save_manifest(
    run: &ChecksumRun,
    request: &ChecksumManifestRequest,
    service: &dyn TransferService,
    cancel: &Cancellation,
) -> Result<ChecksumManifestReceipt, FsError> {
    let path = &request.destination;
    // Separate bounded manifest encoding capacity, shared across all result windows.
    static EXPORTS: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    let _export = EXPORTS
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(2)))
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            failure(
                path,
                io::ErrorKind::WouldBlock,
                "Two checksum manifest exports are already active.",
            )
        })?;
    if !path.is_absolute()
        || path.as_os_str().as_encoded_bytes().contains(&0)
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return Err(failure(
            path,
            io::ErrorKind::InvalidInput,
            "Manifest destination must be an absolute native path without parent steps or NUL.",
        ));
    }
    let parent = path.parent().ok_or_else(|| {
        failure(
            path,
            io::ErrorKind::InvalidInput,
            "Manifest destination needs a containing directory.",
        )
    })?;
    let name = path.file_name().ok_or_else(|| {
        failure(
            path,
            io::ErrorKind::InvalidInput,
            "Manifest destination needs a filename.",
        )
    })?;
    if run.captured_roots.iter().any(|source| source == path)
        || run.rows.iter().any(|row| row.path == *path)
    {
        return Err(failure(
            path,
            io::ErrorKind::InvalidInput,
            "A checksum manifest cannot replace a captured source.",
        ));
    }
    let directory = with_io(cancel, parent, || {
        service.prepare_transfer_destination(parent, cancel)
    })?;
    let device = directory.identity().device;
    if run.rows.iter().any(|row| {
        row.digest(request.algorithm).is_some()
            && !row
                .identity
                .is_some_and(|identity| identity.device == device)
    }) {
        return Err(failure(
            path,
            io::ErrorKind::InvalidInput,
            "Every exported source must have a proven native identity on the manifest's volume.",
        ));
    }
    let source_identities: HashSet<_> = run.rows.iter().filter_map(|row| row.identity).collect();
    if let Some(existing) = with_io(cancel, path, || directory.inspect(name, cancel))? {
        if source_identities.contains(&existing.identity) {
            return Err(failure(
                path,
                io::ErrorKind::InvalidInput,
                "Manifest destination aliases a calculated source.",
            ));
        }
        return Err(failure(
            path,
            io::ErrorKind::AlreadyExists,
            "Manifest destination already exists; choose another filename.",
        ));
    }
    let (bytes, counts) = encode(run, request, parent, cancel)?;
    let source = Box::new(ManifestBytes {
        entry: TransferEntry {
            item: MutationItem {
                path: path.clone(),
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
    let stage = with_io(cancel, path, || directory.stage_file(cancel))?;
    let mut copy = VerifiedFileCopy::new(source, stage)?;
    let mut saved = (|| {
        while copy.phase() != CopyPhase::ReadyToPublish {
            copy.step(cancel)?;
        }
        match copy.publish(name, cancel)? {
            TransferPublication::Published(path) => Ok(ChecksumManifestReceipt {
                path,
                included: counts.included,
                excluded: counts.excluded,
                verified_bytes: copy.verified_bytes(),
            }),
            TransferPublication::PublishedDurabilityUnconfirmed { path, mut error } => {
                error.message = format!(
                    "Manifest published at {}; directory durability is unconfirmed: {}",
                    path.display(),
                    error.message
                );
                Err(error)
            }
            _ => Err(failure(
                path,
                io::ErrorKind::InvalidData,
                "Unexpected replacement outcome for a no-replace manifest.",
            )),
        }
    })();
    if let Err(ref mut primary) = saved
        && let Err(cleanup) = copy.cleanup()
    {
        primary
            .message
            .push_str(&format!(" Staging cleanup also failed: {cleanup}"));
    }
    saved
}

pub(super) fn encode(
    run: &ChecksumRun,
    request: &ChecksumManifestRequest,
    parent: &Path,
    cancel: &Cancellation,
) -> Result<(Vec<u8>, ChecksumPartialAcknowledgement), FsError> {
    let path = &request.destination;
    if !run.algorithms.contains(request.algorithm) {
        return Err(failure(
            path,
            io::ErrorKind::InvalidInput,
            "The chosen algorithm was not part of this captured run.",
        ));
    }
    let included = run
        .rows
        .iter()
        .filter(|row| row.digest(request.algorithm).is_some())
        .count() as u64;
    let counts = ChecksumPartialAcknowledgement {
        included,
        excluded: run.rows.len() as u64 - included,
        incomplete_discovery: matches!(
            run.summary.completion,
            ChecksumCompletion::Incomplete
                | ChecksumCompletion::Cancelled
                | ChecksumCompletion::Failed
        ),
    };
    if included == 0 {
        return Err(failure(
            path,
            io::ErrorKind::InvalidInput,
            "No completed checksums can be exported.",
        ));
    }
    if (counts.excluded > 0 || counts.incomplete_discovery)
        && request.acknowledge_partial != Some(counts)
    {
        return Err(failure(
            path,
            io::ErrorKind::InvalidInput,
            &format!(
                "Partial manifest needs exact acknowledgement: {} included, {} known excluded, incomplete discovery: {}.",
                counts.included, counts.excluded, counts.incomplete_discovery
            ),
        ));
    }
    let mut bytes = Vec::new();
    for row in run.rows.iter() {
        if cancel.is_cancelled() {
            return Err(failure(
                path,
                io::ErrorKind::Interrupted,
                "Manifest export cancelled.",
            ));
        }
        let Some(digest) = row.digest(request.algorithm) else {
            continue;
        };
        if digest.bytes.len() != request.algorithm.bytes() {
            return Err(failure(
                &row.path,
                io::ErrorKind::InvalidData,
                "Malformed completed digest length.",
            ));
        }
        let relative = row.path.strip_prefix(parent).map_err(|_| failure(&row.path, io::ErrorKind::InvalidInput,
            "Manifest sources must be below its containing directory on the same native volume."))?;
        let mut parts = Vec::new();
        for component in relative.components() {
            let Component::Normal(component) = component else {
                return Err(failure(
                    &row.path,
                    io::ErrorKind::InvalidInput,
                    "Manifest names cannot contain traversal, roots or volume prefixes.",
                ));
            };
            parts.push(component.to_str().ok_or_else(|| {
                failure(
                    &row.path,
                    io::ErrorKind::InvalidData,
                    "Native filename cannot round-trip through the UTF-8 GNU manifest format.",
                )
            })?);
        }
        if parts.is_empty() {
            return Err(failure(
                &row.path,
                io::ErrorKind::InvalidInput,
                "Empty relative manifest filename.",
            ));
        }
        let name = parts.join("/");
        let escaped = name.contains(['\\', '\r', '\n']);
        let encoded = if escaped {
            name.replace('\\', "\\\\")
                .replace('\n', "\\n")
                .replace('\r', "\\r")
        } else {
            name
        };
        let line = format!(
            "{}{} *{}\n",
            if escaped { "\\" } else { "" },
            digest.hex(),
            encoded
        );
        let length = bytes
            .len()
            .checked_add(line.len())
            .filter(|length| *length <= MAX_MANIFEST_BYTES)
            .ok_or_else(|| {
                failure(
                    path,
                    io::ErrorKind::OutOfMemory,
                    "Manifest exceeds its bounded 16 MiB encoding budget.",
                )
            })?;
        bytes.try_reserve(length - bytes.len()).map_err(|_| {
            failure(
                path,
                io::ErrorKind::OutOfMemory,
                "Cannot allocate bounded manifest buffer.",
            )
        })?;
        bytes.extend_from_slice(line.as_bytes());
    }
    Ok((bytes, counts))
}

struct ManifestBytes {
    entry: TransferEntry,
    bytes: Arc<[u8]>,
    offset: usize,
}
impl TransferSource for ManifestBytes {
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
                "Manifest export cancelled.",
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
            "A manifest is not a directory.",
        ))
    }
}

#[cfg(test)]
#[path = "checksum_manifest_tests.rs"]
mod tests;
