//! GNU binary checksum and SFV export. Only explicit backend callers may encode/save.
use super::super::verified_output;
use super::format::MAX_MANIFEST_BYTES;
use super::worker::{failure, with_io};
use midnight_core::*;
use std::{
    collections::HashSet,
    io,
    path::{Component, Path},
    sync::{Arc, OnceLock},
};

// A save is a few short native namespace calls under the mutation gate; they keep
// the conservative exclusive disk admission without classifying the destination.
const SAVE: midnight_work_budget::Admission = midnight_work_budget::Admission::Exclusive;

/// Save through exclusive native staging, sync and independent destination SHA-256 readback.
/// An existing destination fails with AlreadyExists unless the caller confirmed `overwrite`;
/// a confirmed one is replaced through the same verified staging and recoverable publication.
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
    let directory = with_io(SAVE, cancel, parent, || {
        service.prepare_transfer_destination(parent, cancel)
    })?;
    let device = directory.identity().device;
    if run.rows.iter().any(|row| {
        export_digest(row, request.algorithm).is_some()
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
    let mut target = None;
    if let Some(existing) = with_io(SAVE, cancel, path, || directory.inspect(name, cancel))? {
        if source_identities.contains(&existing.identity) {
            return Err(failure(
                path,
                io::ErrorKind::InvalidInput,
                "Manifest destination aliases a calculated source.",
            ));
        }
        if !request.overwrite {
            return Err(failure(
                path,
                io::ErrorKind::AlreadyExists,
                "Manifest destination already exists.",
            ));
        }
        let Some(pinned) = with_io(SAVE, cancel, path, || {
            directory.prepare_target(name, cancel)
        })?
        else {
            return Err(failure(
                path,
                io::ErrorKind::NotFound,
                "The manifest destination disappeared; save again.",
            ));
        };
        if pinned.entry().item.kind != EntryKind::File {
            return Err(failure(
                path,
                io::ErrorKind::InvalidInput,
                "The destination is a folder, link or special file and cannot be replaced.",
            ));
        }
        if source_identities.contains(&pinned.entry().identity) {
            return Err(failure(
                path,
                io::ErrorKind::InvalidInput,
                "Manifest destination aliases a calculated source.",
            ));
        }
        target = Some(pinned);
    }
    let (bytes, counts) = encode(run, request, parent, cancel)?;
    let saved = verified_output::publish(directory.as_ref(), path, target.as_ref(), bytes, cancel)?;
    let recovery = saved.recovery;
    if let Some(mut warning) = saved.warning {
        // The manifest is in place, but the save cannot be confirmed: a failure that
        // says so and where a replaced manifest is kept.
        for kept in &recovery {
            warning.message.push_str(&format!(
                "\nThe previous file is kept for recovery at {}",
                kept.display()
            ));
        }
        return Err(warning);
    }
    Ok(ChecksumManifestReceipt {
        path: saved.path,
        included: counts.included,
        excluded: counts.excluded,
        verified_bytes: saved.verified_bytes,
        recovery,
    })
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
        .filter(|row| export_digest(row, request.algorithm).is_some())
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
        let Some(digest) = export_digest(row, request.algorithm) else {
            continue;
        };
        if digest.bytes.len() != request.algorithm.bytes() {
            return Err(failure(
                &row.path,
                io::ErrorKind::InvalidData,
                "Malformed completed digest length.",
            ));
        }
        // The same lines the window copies to the clipboard.
        let line = checksum_file_line(request.algorithm, &digest.hex(), &row.path, parent)
            .map_err(|(kind, message)| failure(&row.path, kind, message))?;
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
    // Publication must not create a manifest that our verifier cannot read, or
    // whose syntax would resolve to a different captured filename. This also
    // applies the same entry, memory and alias limits before native staging.
    let parsed = super::format::parse_manifest(&bytes, path, ChecksumLimits::default(), cancel)?;
    let eligible = run
        .rows
        .iter()
        .filter(|row| export_digest(row, request.algorithm).is_some());
    if parsed.algorithm != request.algorithm
        || parsed.entries.len() as u64 != included
        || parsed.entries.iter().zip(eligible).any(|(entry, row)| {
            entry.path != row.path
                || export_digest(row, request.algorithm)
                    .is_none_or(|digest| entry.digest != digest.bytes)
        })
    {
        return Err(failure(
            path,
            io::ErrorKind::InvalidInput,
            "Checksum filenames cannot round-trip unambiguously through the selected manifest format.",
        ));
    }
    Ok((bytes, counts))
}

fn export_digest(row: &ChecksumRow, algorithm: ChecksumAlgorithm) -> Option<&ChecksumDigest> {
    (row.state == ChecksumRowState::Calculated)
        .then(|| row.digest(algorithm))
        .flatten()
}

#[cfg(test)]
#[path = "checksum_manifest_tests.rs"]
mod tests;
