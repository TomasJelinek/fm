//! GNU binary checksum and SFV export. Only explicit backend callers may encode/save.
//! A manifest of any size is saved: its text follows its rows.
use super::super::verified_output;
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
    // Rows are named below their chunk's base: compare names, without a path per row.
    let replaces_row = run.rows.chunks().iter().any(|chunk| {
        let name = path.strip_prefix(&chunk.base).unwrap_or(path);
        chunk.rows.iter().any(|row| *row.name == *name)
    });
    if run.captured_roots.iter().any(|source| source == path) || replaces_row {
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
        export_digest(&row, request.algorithm).is_some()
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
    // One pass, written straight into a buffer sized for the lines (each name is at most
    // its path below the folder).
    let capacity = run
        .rows
        .iter()
        .filter(|row| export_digest(row, request.algorithm).is_some())
        .fold(0usize, |total, row| {
            total.saturating_add(request.algorithm.bytes() * 2 + 4 + row.name.as_os_str().len())
        });
    let mut text = String::with_capacity(capacity);
    for row in run.rows.iter() {
        if cancel.is_cancelled() {
            return Err(failure(
                path,
                io::ErrorKind::Interrupted,
                "Manifest export cancelled.",
            ));
        }
        let Some(digest) = export_digest(&row, request.algorithm) else {
            continue;
        };
        // The same lines the window copies to the clipboard.
        let name = checksum_file_name(row.base, &row.name, parent).ok_or_else(|| {
            failure(
                &row.path(),
                io::ErrorKind::InvalidInput,
                CHECKSUM_NAME_NOT_BELOW,
            )
        })?;
        push_checksum_file_line(&mut text, digest, &name)
            .map_err(|(kind, message)| failure(&row.path(), kind, message))?;
    }
    let bytes = text.into_bytes();
    // Publication must not create a manifest that our verifier cannot read, or
    // whose syntax would resolve to a different captured filename, or that repeats a
    // name in another letter case.
    let parsed = super::format::parse_manifest(&bytes, path, cancel)?;
    let mut eligible = run
        .rows
        .iter()
        .filter_map(|row| Some((export_digest(&row, request.algorithm)?, row)));
    let mut matches = parsed.algorithm == request.algorithm;
    for (name, expected) in parsed.entries() {
        let Some((digest, row)) = eligible.next() else {
            matches = false;
            break;
        };
        matches &= checksum_file_name(row.base, &row.name, parent).as_deref() == Some(name)
            && digest.bytes == expected;
    }
    if !matches || eligible.next().is_some() {
        return Err(failure(
            path,
            io::ErrorKind::InvalidInput,
            "Checksum filenames cannot round-trip unambiguously through the selected manifest format.",
        ));
    }
    Ok((bytes, counts))
}

fn export_digest<'a>(
    row: &ChecksumRowRef<'a>,
    algorithm: ChecksumAlgorithm,
) -> Option<ChecksumDigest<'a>> {
    (row.row.state == ChecksumRowState::Calculated)
        .then(|| row.row.digest(algorithm))
        .flatten()
}

#[cfg(test)]
#[path = "checksum_manifest_tests.rs"]
mod tests;
