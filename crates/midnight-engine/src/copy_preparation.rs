//! Retry admission without replacing native objects already pinned by an earlier attempt.
//! Construct, attempt and drop only on backend workers. Successful admission releases
//! every cache owner immediately; the Job alone owns its source/capture lifetimes.
use super::*;
use input::Resolved;
use std::{ffi::OsStr, path::PathBuf, sync::MutexGuard};

pub(super) struct Preparation {
    // Failed attempts retain native pins. Destroy those authorities while input
    // still owns the captured provider registration and capture permit.
    service: RetainedService,
    input: CopyInput,
    finished: bool,
}

impl Preparation {
    pub(super) fn new(input: CopyInput, service: Arc<dyn TransferService>) -> Self {
        Self {
            service: RetainedService::new(service),
            input,
            finished: false,
        }
    }

    pub(super) fn attempt(
        &mut self,
        cancel: Cancellation,
        status: Status,
    ) -> Result<worker::Job, FsError> {
        check_cancel(&self.input.destination, &cancel)?;
        if self.finished {
            return Err(error(
                &self.input.destination,
                io::ErrorKind::InvalidInput,
                "Transfer preparation has already completed.",
            ));
        }
        self.service.revalidate(&cancel)?;
        let metadata = self.input.metadata;
        let resolved = self.input.clone().resolve(&self.service, &cancel)?;
        let requested_roots = match &resolved {
            Resolved::Native(request) | Resolved::NativeMove(request) => {
                request.clipboard.paths.len()
            }
            Resolved::Archive { sources, .. } => sources.len(),
            Resolved::Captured { sources, .. } => match sources {
                PreparedCapturedTransfer::Copy(roots) => roots.len(),
                PreparedCapturedTransfer::Move(roots) => roots.len(),
            },
        };
        status
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .requested_roots = requested_roots as u64;
        let mut job = match resolved {
            Resolved::NativeMove(request) => worker::Job::prepare_move_with_metadata(
                request,
                &self.service,
                cancel,
                status,
                metadata,
            ),
            Resolved::Native(request) => {
                worker::Job::prepare_with_metadata(request, &self.service, cancel, status, metadata)
            }
            Resolved::Captured {
                destination,
                sources: PreparedCapturedTransfer::Copy(sources),
            } => worker::Job::prepare_sources(
                destination,
                sources,
                &self.service,
                cancel,
                status,
                metadata,
            ),
            Resolved::Captured {
                destination,
                sources: PreparedCapturedTransfer::Move(sources),
            } => worker::Job::prepare_move_sources(
                destination,
                sources,
                &self.service,
                cancel,
                status,
                metadata,
            ),
            Resolved::Archive {
                destination,
                sources,
            } => worker::Job::prepare_archive(destination, sources, &self.service, cancel, status),
        }?;
        job.set_verification(self.input.verification);
        if let Some(retention) = &self.input._retention {
            job.set_drop_retention(retention.clone());
        }
        self.finished = true;
        // In particular, do not keep the Move source or capture alive until the
        // whole job ends: all source owners must close before stage cleanup.
        self.service.release();
        Ok(job)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Authority {
    Copy,
    Move,
}

#[derive(Default)]
struct Pins {
    sources: HashMap<(PathBuf, Authority), PinnedSource>,
    destinations: HashMap<(PathBuf, Authority), PinnedDirectory>,
}

struct RetainedService {
    service: Arc<dyn TransferService>,
    pins: Mutex<Pins>,
}

impl RetainedService {
    fn new(service: Arc<dyn TransferService>) -> Self {
        Self {
            service,
            pins: Mutex::default(),
        }
    }

    fn pins(&self, path: &Path) -> Result<MutexGuard<'_, Pins>, FsError> {
        self.pins.lock().map_err(|_| {
            error(
                path,
                io::ErrorKind::InvalidData,
                "Retained transfer preparation is unavailable after a worker failure.",
            )
        })
    }

    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        let pins = self.pins(Path::new(""))?;
        for destination in pins.destinations.values() {
            destination.revalidate(cancel)?;
        }
        for source in pins.sources.values() {
            source.revalidate(cancel)?;
        }
        Ok(())
    }

    fn release(&mut self) {
        let pins = self
            .pins
            .get_mut()
            .unwrap_or_else(|error| error.into_inner());
        *pins = Pins::default();
    }

    fn source(
        &self,
        path: &Path,
        authority: Authority,
        cancel: &Cancellation,
    ) -> Result<PinnedSource, FsError> {
        check_cancel(path, cancel)?;
        let key = (path.to_owned(), authority);
        let previous = self.pins(path)?.sources.get(&key).cloned();
        let source = if let Some(previous) = previous {
            previous
        } else {
            let (source, capture) = match authority {
                Authority::Copy => (self.service.prepare_transfer_source(path, cancel)?, None),
                Authority::Move => {
                    let prepared = self.service.prepare_move_source(path, cancel)?;
                    (prepared.source, Some(prepared.capture))
                }
            };
            let source = PinnedSource {
                source: SharedSource::new(source),
                capture,
            };
            self.pins(path)?.sources.insert(key, source.clone());
            source
        };
        source.revalidate(cancel)?;
        Ok(source)
    }

    fn destination(
        &self,
        path: &Path,
        authority: Authority,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        check_cancel(path, cancel)?;
        let key = (path.to_owned(), authority);
        let previous = self.pins(path)?.destinations.get(&key).cloned();
        let destination = if let Some(previous) = previous {
            previous
        } else {
            let directory = match authority {
                Authority::Copy => self.service.prepare_transfer_destination(path, cancel)?,
                Authority::Move => self.service.prepare_move_destination(path, cancel)?,
            };
            let destination = PinnedDirectory {
                path: path.to_owned(),
                identity: directory.identity(),
                directory,
            };
            self.pins(path)?
                .destinations
                .insert(key, destination.clone());
            destination
        };
        destination.revalidate(cancel)?;
        Ok(destination.directory)
    }
}

impl TransferService for RetainedService {
    fn prepare_checksum_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.service.prepare_checksum_directory(path, cancel)
    }

    fn prepare_transfer_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        Ok(Box::new(self.source(path, Authority::Copy, cancel)?.source))
    }

    fn prepare_transfer_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.destination(path, Authority::Copy, cancel)
    }

    fn prepare_move_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<PreparedTransferMove, FsError> {
        let source = self.source(path, Authority::Move, cancel)?;
        let capture = source.capture.ok_or_else(|| {
            error(
                path,
                io::ErrorKind::InvalidData,
                "Retained Move source has no mutation authority.",
            )
        })?;
        Ok(PreparedTransferMove {
            source: Box::new(source.source),
            capture,
        })
    }

    fn prepare_move_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.destination(path, Authority::Move, cancel)
    }
}

#[derive(Clone)]
struct PinnedDirectory {
    path: PathBuf,
    identity: TransferIdentity,
    directory: Arc<dyn TransferDirectory>,
}

impl PinnedDirectory {
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        check_cancel(&self.path, cancel)?;
        self.directory.revalidate(cancel)?;
        if self.directory.path() != self.path || self.directory.identity() != self.identity {
            return Err(error(
                &self.path,
                io::ErrorKind::InvalidData,
                "Retained transfer destination changed identity.",
            ));
        }
        Ok(())
    }
}

#[derive(Clone)]
struct PinnedSource {
    source: SharedSource,
    capture: Option<Arc<dyn TransferMoveCapture>>,
}

impl PinnedSource {
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.source.revalidate(cancel)?;
        if let Some(capture) = &self.capture {
            capture.revalidate(cancel)?;
            if !same_entry(capture.entry(), self.source.entry()) {
                return Err(error(
                    &self.source.entry.item.path,
                    io::ErrorKind::InvalidData,
                    "Retained Move source and mutation authority no longer match.",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
struct SharedSource {
    entry: TransferEntry,
    native_identity: Option<TransferIdentity>,
    virtual_identity: Option<VirtualEntryId>,
    source: Arc<Mutex<Box<dyn TransferSource>>>,
}

impl SharedSource {
    fn new(source: Box<dyn TransferSource>) -> Self {
        Self {
            entry: source.entry().clone(),
            native_identity: source.native_identity(),
            virtual_identity: source.virtual_identity(),
            source: Arc::new(Mutex::new(source)),
        }
    }

    fn source(&self) -> Result<MutexGuard<'_, Box<dyn TransferSource>>, FsError> {
        self.source.lock().map_err(|_| {
            error(
                &self.entry.item.path,
                io::ErrorKind::InvalidData,
                "Retained source is unavailable after a worker failure.",
            )
        })
    }
}

impl TransferSource for SharedSource {
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }

    fn native_identity(&self) -> Option<TransferIdentity> {
        self.native_identity
    }

    fn tree_copy_occurrence(&self) -> Option<TreeCopyOccurrence> {
        self.source()
            .ok()
            .and_then(|source| source.tree_copy_occurrence())
    }
    fn virtual_identity(&self) -> Option<VirtualEntryId> {
        self.virtual_identity
    }

    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        check_cancel(&self.entry.item.path, cancel)?;
        let source = self.source()?;
        source.revalidate(cancel)?;
        if !same_entry(source.entry(), &self.entry)
            || source.native_identity() != self.native_identity
            || source.virtual_identity() != self.virtual_identity
        {
            return Err(error(
                &self.entry.item.path,
                io::ErrorKind::InvalidData,
                "Retained transfer source changed identity or revision.",
            ));
        }
        Ok(())
    }

    fn open_child(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.source()?.open_child(name, cancel)
    }

    fn next_move_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<PreparedTransferMove>, FsError> {
        self.source()?.next_move_child(cancel)
    }

    fn prepare_modified_time(
        &self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferModifiedTimePin>, FsError> {
        self.source()?.prepare_modified_time(cancel)
    }

    fn rewind_for_validation(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferReadGuard>, FsError> {
        self.source()?.rewind_for_validation(cancel)
    }

    fn seek_for_validation(&mut self, offset: u64, cancel: &Cancellation) -> Result<u64, FsError> {
        self.source()?.seek_for_validation(offset, cancel)
    }

    fn read_chunk(&mut self, buffer: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        self.source()?.read_chunk(buffer, cancel)
    }

    fn supports_read_retry(&self) -> bool {
        self.source()
            .is_ok_and(|source| source.supports_read_retry())
    }

    fn retry_read_from(&mut self, offset: u64, cancel: &Cancellation) -> Result<(), FsError> {
        self.source()?.retry_read_from(offset, cancel)
    }

    fn next_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        self.source()?.next_child(cancel)
    }
}

fn same_entry(left: &TransferEntry, right: &TransferEntry) -> bool {
    left.identity == right.identity
        && left.item.path == right.item.path
        && left.item.kind == right.item.kind
        && left.item.metadata == right.item.metadata
}

fn check_cancel(path: &Path, cancel: &Cancellation) -> Result<(), FsError> {
    if cancel.is_cancelled() {
        Err(error(
            path,
            io::ErrorKind::Interrupted,
            "Transfer cancelled.",
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "copy_preparation_tests.rs"]
mod tests;
