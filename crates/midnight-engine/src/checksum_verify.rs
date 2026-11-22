//! Manifest parsing and verification stay inside the existing serial checksum worker.
//! A manifest of any size is read as a stream: memory follows its entries.
use super::{format, worker};
use midnight_core::*;
use std::{
    io,
    path::{Component, Path},
    sync::Arc,
};

pub(super) fn verify(
    request: ChecksumVerifyRequest,
    service: &dyn TransferService,
    storage: &dyn worker::Storage,
    cancel: &Cancellation,
    retention: Arc<ChecksumRetention>,
    mut emit: impl FnMut(ChecksumUpdate) -> bool,
) -> Arc<ChecksumRun> {
    let prepared = prepare(&request, service, storage, cancel);
    let (directory, manifest, ssd) = match prepared {
        Ok(value) => value,
        Err(error) => {
            return Arc::new(ChecksumRun {
                job: request.job,
                generation: request.generation,
                algorithms: ChecksumAlgorithms::default(),
                captured_roots: vec![request.manifest].into(),
                rows: ChecksumRows::default(),
                summary: ChecksumSummary {
                    completion: if cancel.is_cancelled() {
                        ChecksumCompletion::Cancelled
                    } else {
                        ChecksumCompletion::Failed
                    },
                    progress: ChecksumProgress {
                        errors: u64::from(!cancel.is_cancelled()),
                        ..Default::default()
                    },
                    diagnostics: vec![error],
                },
                retention,
            });
        }
    };
    let algorithm = manifest.algorithm;
    if !emit(ChecksumUpdate::Verifying(algorithm)) {
        cancel.cancel();
    }
    let algorithms = match ChecksumAlgorithms::new(&[algorithm]) {
        Ok(value) => value,
        Err(_) => unreachable!("a parsed algorithm is nonempty"),
    };
    let folder: Arc<Path> = directory.path().into();
    let provider = Contained { directory };
    let mut transformed = ChecksumRows::default();
    let mut counts = ChecksumProgress::default();
    let calculation = ChecksumRequest {
        job: request.job,
        generation: request.generation,
        targets: Vec::new(),
        algorithms,
        recursive: false,
    };
    let run = worker::calculate_for_verification(
        &calculation,
        folder,
        &manifest.names,
        request.manifest.clone(),
        &provider,
        // Every entry is confined to the manifest directory's volume.
        &worker::Known(ssd),
        cancel,
        retention,
        |update| match update {
            ChecksumUpdate::Batch(chunk) => {
                let chunk = Arc::new(compare(&chunk, &manifest, algorithm, &mut counts));
                transformed.push(chunk.clone());
                emit(ChecksumUpdate::Batch(chunk))
            }
            ChecksumUpdate::Progress(mut progress) => {
                set_counts(&mut progress, &counts);
                emit(ChecksumUpdate::Progress(progress))
            }
            other => emit(other),
        },
    );
    let mut run = (*run).clone();
    // Every row went out in a batch: the compared chunks are the run's rows.
    run.rows = transformed;
    set_counts(&mut run.summary.progress, &counts);
    if run.summary.completion == ChecksumCompletion::Completed
        && (counts.mismatched > 0 || counts.missing > 0)
    {
        run.summary.completion = ChecksumCompletion::CompletedWithErrors;
    }
    Arc::new(run)
}

/// The chunk's rows with their verdicts. Each entry gives exactly one row, in file order,
/// so a row's ID names its entry; a row whose name is not that entry's is a mismatch.
fn compare(
    chunk: &ChecksumChunk,
    manifest: &format::ParsedManifest,
    algorithm: ChecksumAlgorithm,
    counts: &mut ChecksumProgress,
) -> ChecksumChunk {
    let rows = chunk
        .rows
        .iter()
        .map(|row| {
            let mut row = row.clone();
            if row.state == ChecksumRowState::Calculated {
                let entry = row
                    .id
                    .0
                    .checked_sub(1)
                    .and_then(|index| usize::try_from(index).ok());
                let expected = entry.and_then(|index| {
                    (manifest.names.get(index)? == &row.name)
                        .then(|| manifest.digest(index))
                        .flatten()
                });
                let actual = row.digest(algorithm).map(|digest| digest.bytes);
                if expected.is_some() && expected == actual {
                    row.state = ChecksumRowState::Verified;
                    counts.verified += 1;
                } else {
                    row.state = ChecksumRowState::Mismatch;
                    counts.mismatched += 1;
                }
            } else if row.state == ChecksumRowState::ReadError
                && row
                    .error
                    .as_ref()
                    .is_some_and(|error| error.kind == io::ErrorKind::NotFound)
            {
                row.state = ChecksumRowState::Missing;
                counts.missing += 1;
            }
            row
        })
        .collect();
    ChecksumChunk {
        base: chunk.base.clone(),
        rows,
        retention: chunk.retention.clone(),
    }
}

fn set_counts(progress: &mut ChecksumProgress, counts: &ChecksumProgress) {
    progress.verified = counts.verified;
    progress.mismatched = counts.mismatched;
    progress.missing = counts.missing;
}

/// Also returns whether the manifest directory is a confirmed local SSD.
fn prepare(
    request: &ChecksumVerifyRequest,
    service: &dyn TransferService,
    storage: &dyn worker::Storage,
    cancel: &Cancellation,
) -> Result<(Arc<dyn TransferDirectory>, format::ParsedManifest, bool), FsError> {
    request.validate().map_err(|message| {
        worker::failure(&request.manifest, io::ErrorKind::InvalidInput, message)
    })?;
    let path = &request.manifest;
    let parent = path.parent().ok_or_else(|| {
        worker::failure(
            path,
            io::ErrorKind::InvalidInput,
            "Manifest requires a parent directory.",
        )
    })?;
    let name = path.file_name().ok_or_else(|| {
        worker::failure(
            path,
            io::ErrorKind::InvalidInput,
            "Manifest requires a filename.",
        )
    })?;
    let ssd = storage.confirmed_local_ssd(parent);
    let admission = midnight_work_budget::storage_admission(ssd);
    let directory = worker::with_io(admission, cancel, parent, || {
        service.prepare_checksum_directory(parent, cancel)
    })?;
    let mut source = worker::with_io(admission, cancel, path, || {
        directory.open_read_child(name, cancel)
    })?;
    let entry = source.entry().clone();
    if entry.item.path != *path
        || entry.item.kind != EntryKind::File
        || source.native_identity().is_none()
    {
        return Err(worker::failure(
            path,
            io::ErrorKind::InvalidInput,
            "Checksum manifest must be an ordinary native file.",
        ));
    }
    // The listed length detects a file that changes while it is read; it is no limit.
    let length = entry.item.metadata.size.ok_or_else(|| {
        worker::failure(
            path,
            io::ErrorKind::InvalidData,
            "Manifest has no known length.",
        )
    })?;
    let mut parser = format::ManifestParser::new(path, cancel)?;
    let mut read = 0u64;
    let mut buffer = vec![0; CHECKSUM_BUFFER_BYTES];
    let revalidate = |source: &dyn TransferSource| {
        worker::with_io(admission, cancel, path, || {
            directory.revalidate(cancel)?;
            source.revalidate(cancel)
        })
    };
    // Same contract as calculation: check before the first and after the last read,
    // in between only at the bounded cadence (the pinned handle cannot switch files).
    revalidate(source.as_ref())?;
    let mut cadence = worker::Cadence::new(worker::REVALIDATION_INTERVAL);
    loop {
        let count = match worker::with_io(admission, cancel, path, || {
            source.read_chunk(&mut buffer, cancel)
        }) {
            Err(error) if error.kind == io::ErrorKind::Interrupted && !cancel.is_cancelled() => {
                continue;
            }
            result => result?,
        };
        if count > buffer.len() || count as u64 > length.saturating_sub(read) {
            return Err(worker::failure(
                path,
                io::ErrorKind::InvalidData,
                "Manifest length changed or the native read returned an invalid length.",
            ));
        }
        if count == 0 {
            break;
        }
        read += count as u64;
        // Each chunk is parsed as it arrives: only the line being read is held.
        parser.feed(&buffer[..count])?;
        if cadence.due() {
            revalidate(source.as_ref())?;
        }
    }
    revalidate(source.as_ref())?;
    if read != length {
        return Err(worker::failure(
            path,
            io::ErrorKind::InvalidData,
            "Manifest length changed while reading.",
        ));
    }
    let parsed = parser.finish()?;
    revalidate(source.as_ref())?;
    Ok((directory, parsed, ssd))
}

struct Contained {
    directory: Arc<dyn TransferDirectory>,
}
impl TransferService for Contained {
    fn prepare_transfer_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        let invalid = || {
            worker::failure(
                path,
                io::ErrorKind::InvalidInput,
                "Manifest entry is not a safe relative native path.",
            )
        };
        let relative = path
            .strip_prefix(self.directory.path())
            .map_err(|_| invalid())?;
        let mut components = relative.components();
        let Some(Component::Normal(name)) = components.next() else {
            return Err(invalid());
        };
        self.directory.revalidate(cancel)?;
        let context = |mut error: FsError| {
            error.path = path.to_owned();
            error
        };
        let mut source = self
            .directory
            .open_read_child(name, cancel)
            .map_err(context)?;
        let mut ancestors = Vec::new();
        for component in components {
            let Component::Normal(name) = component else {
                return Err(invalid());
            };
            if source.entry().identity.device != self.directory.identity().device {
                return Err(invalid());
            }
            let next = source.open_child(name, cancel).map_err(context)?;
            ancestors.push(source);
            source = next;
        }
        if source.entry().item.path != path
            || source.entry().identity.device != self.directory.identity().device
            || source.native_identity().is_none()
        {
            return Err(invalid());
        }
        Ok(Box::new(PinnedSource {
            directory: self.directory.clone(),
            ancestors,
            source,
        }))
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        Err(worker::failure(
            path,
            io::ErrorKind::Unsupported,
            "Checksum verification has no write destination.",
        ))
    }
}
struct PinnedSource {
    directory: Arc<dyn TransferDirectory>,
    ancestors: Vec<Box<dyn TransferSource>>,
    source: Box<dyn TransferSource>,
}
impl TransferSource for PinnedSource {
    fn entry(&self) -> &TransferEntry {
        self.source.entry()
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.directory.revalidate(cancel)?;
        for ancestor in &self.ancestors {
            ancestor.revalidate(cancel)?;
        }
        self.source.revalidate(cancel)
    }
    fn read_chunk(&mut self, buffer: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        self.source.read_chunk(buffer, cancel)
    }
    fn next_child(&mut self, _: &Cancellation) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        Err(worker::failure(
            &self.entry().item.path,
            io::ErrorKind::Unsupported,
            "Verification only reads listed files.",
        ))
    }
}

#[cfg(test)]
#[path = "checksum_verify_tests.rs"]
mod tests;
