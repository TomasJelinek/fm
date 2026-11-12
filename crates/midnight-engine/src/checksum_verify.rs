//! Manifest parsing and verification stay inside the existing serial checksum worker.
use super::{format, worker};
use midnight_core::*;
use std::{
    collections::HashMap,
    io,
    path::{Component, Path},
    sync::Arc,
};

pub(super) fn verify(
    request: ChecksumVerifyRequest,
    service: &dyn TransferService,
    cancel: &Cancellation,
    retention: Arc<ChecksumRetention>,
    mut emit: impl FnMut(ChecksumUpdate) -> bool,
) -> Arc<ChecksumRun> {
    let prepared = prepare(&request, service, cancel);
    let (directory, manifest) = match prepared {
        Ok(value) => value,
        Err(error) => {
            return Arc::new(ChecksumRun {
                job: request.job,
                generation: request.generation,
                algorithms: ChecksumAlgorithms::default(),
                captured_roots: vec![request.manifest].into(),
                rows: Arc::from([]),
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
                    reached_budgets: vec![],
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
    let targets = manifest
        .entries
        .iter()
        .map(|entry| entry.path.clone().into())
        .collect();
    let expected: HashMap<_, _> = manifest
        .entries
        .into_iter()
        .map(|entry| (entry.path, entry.digest))
        .collect();
    let provider = Contained { directory };
    let mut transformed = HashMap::new();
    let mut counts = ChecksumProgress::default();
    let run = worker::calculate(
        ChecksumRequest {
            job: request.job,
            generation: request.generation,
            targets,
            algorithms,
            recursive: false,
            limits: request.limits,
        },
        &provider,
        cancel,
        retention,
        |update| match update {
            ChecksumUpdate::Batch(rows) => {
                let rows: Arc<[_]> = rows
                    .iter()
                    .map(|row| {
                        let mut row = (**row).clone();
                        if row.state == ChecksumRowState::Calculated {
                            if row.digest(algorithm).is_some_and(|actual| {
                                expected.get(&row.path) == Some(&actual.bytes)
                            }) {
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
                        let row = Arc::new(row);
                        transformed.insert(row.id, row.clone());
                        row
                    })
                    .collect();
                emit(ChecksumUpdate::Batch(rows))
            }
            ChecksumUpdate::Progress(mut progress) => {
                set_counts(&mut progress, &counts);
                emit(ChecksumUpdate::Progress(progress))
            }
            other => emit(other),
        },
    );
    let mut run = (*run).clone();
    run.rows = run
        .rows
        .iter()
        .map(|row| transformed.remove(&row.id).unwrap_or_else(|| row.clone()))
        .collect();
    set_counts(&mut run.summary.progress, &counts);
    if run.summary.completion == ChecksumCompletion::Completed
        && (counts.mismatched > 0 || counts.missing > 0)
    {
        run.summary.completion = ChecksumCompletion::CompletedWithErrors;
    }
    Arc::new(run)
}

fn set_counts(progress: &mut ChecksumProgress, counts: &ChecksumProgress) {
    progress.verified = counts.verified;
    progress.mismatched = counts.mismatched;
    progress.missing = counts.missing;
}

fn prepare(
    request: &ChecksumVerifyRequest,
    service: &dyn TransferService,
    cancel: &Cancellation,
) -> Result<(Arc<dyn TransferDirectory>, format::ParsedManifest), FsError> {
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
    let directory = worker::with_io(cancel, parent, || {
        service.prepare_checksum_directory(parent, cancel)
    })?;
    let mut source = worker::with_io(cancel, path, || directory.open_read_child(name, cancel))?;
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
    let maximum = format::MAX_MANIFEST_BYTES.min(request.limits.max_retained_bytes);
    let length = entry
        .item
        .metadata
        .size
        .filter(|length| *length <= maximum as u64)
        .ok_or_else(|| {
            worker::failure(
                path,
                io::ErrorKind::InvalidData,
                "Manifest exceeds its encoded input budget or has no known length.",
            )
        })?;
    let mut bytes = Vec::with_capacity(length as usize);
    let mut buffer = vec![0; CHECKSUM_BUFFER_BYTES];
    loop {
        worker::with_io(cancel, path, || {
            directory.revalidate(cancel)?;
            source.revalidate(cancel)
        })?;
        let count = match worker::with_io(cancel, path, || source.read_chunk(&mut buffer, cancel)) {
            Err(error) if error.kind == io::ErrorKind::Interrupted && !cancel.is_cancelled() => {
                continue;
            }
            result => result?,
        };
        if count > buffer.len() || count > (length as usize).saturating_sub(bytes.len()) {
            return Err(worker::failure(
                path,
                io::ErrorKind::InvalidData,
                "Manifest length changed or the native read returned an invalid length.",
            ));
        }
        worker::with_io(cancel, path, || {
            directory.revalidate(cancel)?;
            source.revalidate(cancel)
        })?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    if bytes.len() != length as usize {
        return Err(worker::failure(
            path,
            io::ErrorKind::InvalidData,
            "Manifest length changed while reading.",
        ));
    }
    let parsed = format::parse_manifest(&bytes, path, request.limits, cancel)?;
    worker::with_io(cancel, path, || {
        directory.revalidate(cancel)?;
        source.revalidate(cancel)
    })?;
    Ok((directory, parsed))
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
