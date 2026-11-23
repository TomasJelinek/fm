//! Export and Import configuration on bounded backend workers, one settings file
//! at a time. Export validates and encodes the whole snapshot, then publishes it
//! through verified native staging while holding the namespace gate; an existing
//! destination needs explicit consent and a replaced file is kept for recovery.
//! Import reads one ordinary native file within the settings byte budget and
//! validates all of it. Neither changes the application's own settings.
use crate::{Outbox, task_lifetime::TaskLifetime};
use midnight_core::*;
use midnight_platform_common::{
    settings::{MAX_PREFERENCES_BYTES, decode_settings_file, encode_settings_file},
    transfer::{CopyPhase, VerifiedFileCopy},
};
use std::{
    ffi::OsStr,
    io,
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore},
    task::JoinSet,
};

/// A few short native namespace calls with the conservative exclusive disk admission.
const IO: midnight_work_budget::Admission = midnight_work_budget::Admission::Exclusive;
const READ_CHUNK: usize = 64 * 1024;

pub(crate) struct Controller {
    service: Option<Arc<dyn TransferService>>,
    mutation: Arc<Semaphore>,
    budget: Arc<Semaphore>,
    /// Output windows' Save as…, independent of settings files.
    outputs: Arc<Semaphore>,
    shutdown: Cancellation,
    outbox: Outbox,
    scope: TaskLifetime,
}

impl Controller {
    pub(crate) fn new(
        scope: TaskLifetime,
        service: Option<Arc<dyn TransferService>>,
        mutation: Arc<Semaphore>,
        outbox: Outbox,
    ) -> Self {
        Self {
            service,
            mutation,
            budget: Arc::new(Semaphore::new(1)),
            outputs: Arc::new(Semaphore::new(2)),
            shutdown: Cancellation::default(),
            outbox,
            scope,
        }
    }

    pub(crate) async fn export(
        &self,
        input: Box<PreferencesExportRequest>,
        tasks: &mut JoinSet<()>,
    ) {
        let request = input.request;
        let (permit, service) = match self.admit(&input.destination, "export settings") {
            Ok(admitted) => admitted,
            Err(error) => {
                let result = Err(error);
                self.outbox
                    .send(Event::PreferencesExported { request, result })
                    .await;
                return;
            }
        };
        let mutation = self.mutation.clone();
        let cancel = self.shutdown.child();
        let outbox = self.outbox.clone();
        let scope = self.scope.clone();
        tasks.spawn(self.scope.future(async move {
            // Admission stays owned until the reply has been queued.
            let _permit = permit;
            let result = export_file(*input, service, mutation, &cancel, &scope).await;
            if !cancel.is_cancelled() {
                outbox
                    .send(Event::PreferencesExported { request, result })
                    .await;
            }
        }));
    }

    /// Save as… of an output window (`Command::SaveUserCommandOutput`).
    pub(crate) async fn save_output(
        &self,
        input: Box<UserCommandOutputSaveRequest>,
        tasks: &mut JoinSet<()>,
    ) {
        let request = input.request;
        let admitted = split(&input.destination, OUTPUT.operation).and_then(|_| {
            let service = self.service.clone().ok_or_else(|| {
                failure(
                    &input.destination,
                    OUTPUT.operation,
                    io::ErrorKind::Unsupported,
                    "This platform cannot save files.",
                )
            })?;
            let permit = self.outputs.clone().try_acquire_owned().map_err(|_| {
                failure(
                    &input.destination,
                    OUTPUT.operation,
                    io::ErrorKind::WouldBlock,
                    "Two command outputs are being saved. Try again when one has finished.",
                )
            })?;
            Ok((permit, service))
        });
        let (permit, service) = match admitted {
            Ok(admitted) => admitted,
            Err(error) => {
                let result = Err(error);
                self.outbox
                    .send(Event::UserCommandOutputSaved { request, result })
                    .await;
                return;
            }
        };
        let mutation = self.mutation.clone();
        let cancel = self.shutdown.child();
        let outbox = self.outbox.clone();
        let scope = self.scope.clone();
        tasks.spawn(self.scope.future(async move {
            // Admission stays owned until the reply has been queued.
            let _permit = permit;
            let result = save_output(*input, service, mutation, &cancel, &scope).await;
            if !cancel.is_cancelled() {
                outbox
                    .send(Event::UserCommandOutputSaved { request, result })
                    .await;
            }
        }));
    }

    pub(crate) fn import(
        &self,
        request: DesktopRequestId,
        source: PathBuf,
        tasks: &mut JoinSet<()>,
    ) {
        let admitted = self.admit(&source, "import settings");
        let cancel = self.shutdown.child();
        let shutdown = self.shutdown.clone();
        let outbox = self.outbox.clone();
        tasks.spawn_blocking(self.scope.blocking(move || {
            // Admission stays owned until the reply has been queued.
            let (result, _permit) = match admitted {
                Ok((permit, service)) => (
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        import_file(&source, service.as_ref(), &cancel)
                    }))
                    .unwrap_or_else(|_| {
                        Err(failure(
                            &source,
                            "import settings",
                            io::ErrorKind::Other,
                            "The settings import stopped unexpectedly.",
                        ))
                    }),
                    Some(permit),
                ),
                Err(error) => (Err(error), None),
            };
            let result = result.map(Box::new);
            outbox.blocking_send(Event::PreferencesImported { request, result }, &shutdown);
        }));
    }

    pub(crate) fn shutdown(&self) {
        self.shutdown.cancel();
    }

    fn admit(
        &self,
        path: &Path,
        operation: &'static str,
    ) -> Result<(OwnedSemaphorePermit, Arc<dyn TransferService>), FsError> {
        split(path, operation)?;
        let service = self.service.clone().ok_or_else(|| {
            failure(
                path,
                operation,
                io::ErrorKind::Unsupported,
                "This platform cannot read or write settings files.",
            )
        })?;
        let permit = self.budget.clone().try_acquire_owned().map_err(|_| {
            failure(
                path,
                operation,
                io::ErrorKind::WouldBlock,
                "Another settings file is being exported or imported.",
            )
        })?;
        Ok((permit, service))
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        self.shutdown();
    }
}

async fn export_file(
    input: PreferencesExportRequest,
    service: Arc<dyn TransferService>,
    mutation: Arc<Semaphore>,
    cancel: &Cancellation,
    scope: &TaskLifetime,
) -> Result<PreferencesExportReceipt, FsError> {
    let destination = input.destination;
    let path = destination.clone();
    // Validate and encode the whole snapshot before any namespace work.
    let bytes = tokio::task::spawn_blocking(scope.blocking(move || {
        encode_settings_file(input.preferences)
            .map_err(|error| FsError::from_io("export settings", &path, error))
    }))
    .await
    .unwrap_or_else(|_| Err(worker_failure(&destination)))?;
    publish_gated(
        destination,
        bytes.into(),
        input.overwrite,
        service,
        mutation,
        cancel,
        scope,
        &SETTINGS,
    )
    .await
}

/// `publish` while holding the namespace gate, queued in FIFO order.
#[allow(clippy::too_many_arguments)]
async fn publish_gated(
    destination: PathBuf,
    bytes: Arc<[u8]>,
    overwrite: bool,
    service: Arc<dyn TransferService>,
    mutation: Arc<Semaphore>,
    cancel: &Cancellation,
    scope: &TaskLifetime,
    what: &'static Published,
) -> Result<PreferencesExportReceipt, FsError> {
    // One queued acquisition keeps this export's FIFO place at the namespace gate.
    let acquire = mutation.acquire_many_owned(crate::transfer_leases::NAMESPACE_WEIGHT);
    tokio::pin!(acquire);
    let gate = loop {
        if cancel.is_cancelled() {
            return Err(cancelled(&destination, what.operation));
        }
        tokio::select! {
            permit = &mut acquire => break permit.map_err(|_| failure(
                &destination,
                what.operation,
                io::ErrorKind::BrokenPipe,
                "The mutation service is closing.",
            ))?,
            // Cancellation is a polled flag; check it while queued.
            () = tokio::time::sleep(Duration::from_millis(25)) => {}
        }
    };
    let cancel = cancel.clone();
    let path = destination.clone();
    tokio::task::spawn_blocking(scope.blocking(move || {
        let result = publish_as(&path, bytes, overwrite, service.as_ref(), &cancel, what);
        // Native cleanup has finished before the namespace gate is released.
        drop(gate);
        result
    }))
    .await
    .unwrap_or_else(|_| Err(worker_failure(&destination)))
}

const OUTPUT: Published = Published {
    operation: "save command output",
    exists: "A file with this name already exists.",
    noun: "output",
};

/// Save as… of an output window: the session's text, published like a settings export.
async fn save_output(
    input: UserCommandOutputSaveRequest,
    service: Arc<dyn TransferService>,
    mutation: Arc<Semaphore>,
    cancel: &Cancellation,
    scope: &TaskLifetime,
) -> Result<UserCommandOutputSaveReceipt, FsError> {
    let session = input.session.clone();
    let bytes: Arc<[u8]> =
        tokio::task::spawn_blocking(scope.blocking(move || Arc::from(session.text().into_bytes())))
            .await
            .map_err(|_| worker_failure(&input.destination))?;
    let receipt = publish_gated(
        input.destination,
        bytes,
        input.overwrite,
        service,
        mutation,
        cancel,
        scope,
        &OUTPUT,
    )
    .await?;
    Ok(UserCommandOutputSaveReceipt {
        path: receipt.path,
        bytes: receipt.bytes,
        recovery: receipt.recovery,
    })
}

/// What a published file is, in its errors.
struct Published {
    operation: &'static str,
    exists: &'static str,
    noun: &'static str,
}
const SETTINGS: Published = Published {
    operation: "export settings",
    exists: "The settings file already exists.",
    noun: "settings file",
};

/// The settings export's `publish_as`, as its tests call it.
#[cfg(test)]
fn publish(
    destination: &Path,
    bytes: Arc<[u8]>,
    overwrite: bool,
    service: &dyn TransferService,
    cancel: &Cancellation,
) -> Result<PreferencesExportReceipt, FsError> {
    publish_as(destination, bytes, overwrite, service, cancel, &SETTINGS)
}

/// Publish through exclusive staging, sync and independent readback. An existing
/// regular file is replaced only with consent, keeping the displaced file.
fn publish_as(
    destination: &Path,
    bytes: Arc<[u8]>,
    overwrite: bool,
    service: &dyn TransferService,
    cancel: &Cancellation,
    what: &Published,
) -> Result<PreferencesExportReceipt, FsError> {
    let operation = what.operation;
    let (parent, name) = split(destination, operation)?;
    let directory = with_io(destination, operation, cancel, || {
        service.prepare_transfer_destination(parent, cancel)
    })?;
    let target = with_io(destination, operation, cancel, || {
        directory.prepare_target(name, cancel)
    })?;
    if let Some(target) = &target {
        if !overwrite {
            return Err(failure(
                destination,
                operation,
                io::ErrorKind::AlreadyExists,
                what.exists,
            ));
        }
        if target.entry().item.kind != EntryKind::File {
            return Err(failure(
                destination,
                operation,
                io::ErrorKind::InvalidInput,
                "The destination is a folder, link or special file and cannot be replaced.",
            ));
        }
    }
    let source = Box::new(SettingsBytes {
        entry: TransferEntry {
            item: MutationItem {
                path: destination.to_owned(),
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
        bytes,
        offset: 0,
    });
    let stage = with_io(destination, operation, cancel, || match &target {
        Some(target) => {
            let capabilities = directory.capabilities();
            let policy = if capabilities.conditional_overwrite {
                TransferReplacementPolicy::ConditionalIdentity
            } else if capabilities.recoverable_replacement {
                TransferReplacementPolicy::RecoverableCurrentEntry
            } else {
                return Err(failure(
                    destination,
                    operation,
                    io::ErrorKind::Unsupported,
                    "This filesystem does not support safely replacing a file.",
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
        let publication = match &target {
            Some(target) => {
                with_io(destination, operation, cancel, || target.revalidate(cancel))?;
                copy.publish_replace(name, cancel)?
            }
            None => copy.publish(name, cancel)?,
        };
        receipt(publication, copy.verified_bytes(), what.noun)
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

fn receipt(
    publication: TransferPublication,
    bytes: u64,
    noun: &str,
) -> Result<PreferencesExportReceipt, FsError> {
    match publication {
        TransferPublication::Published(path) => Ok(PreferencesExportReceipt {
            path,
            bytes,
            recovery: Vec::new(),
        }),
        TransferPublication::Replaced { path, recovery } => Ok(PreferencesExportReceipt {
            path,
            bytes,
            recovery: vec![recovery.path],
        }),
        TransferPublication::PublishedDurabilityUnconfirmed { path, mut error } => {
            error.message = format!(
                "The {noun} was written to {}, but its folder could not confirm \
                 that it is stored durably: {}",
                path.display(),
                error.message
            );
            Err(error)
        }
        TransferPublication::RecoveryRequired {
            recovery,
            mut error,
            ..
        } => {
            for item in recovery {
                error
                    .message
                    .push_str(&format!("\nRecovery file: {}", item.path.display()));
            }
            Err(error)
        }
    }
}

/// Read one ordinary native file through its pinned no-follow parent, rechecking
/// its identity around the bounded read, then decode and validate all of it.
fn import_file(
    path: &Path,
    service: &dyn TransferService,
    cancel: &Cancellation,
) -> Result<ImportedPreferences, FsError> {
    const OPERATION: &str = "import settings";
    let (parent, name) = split(path, OPERATION)?;
    let directory = with_io(path, OPERATION, cancel, || {
        service.prepare_checksum_directory(parent, cancel)
    })?;
    let mut source = with_io(path, OPERATION, cancel, || {
        directory.open_read_child(name, cancel)
    })?;
    let entry = source.entry().clone();
    if entry.item.path.as_path() != path
        || entry.item.kind != EntryKind::File
        || source.native_identity().is_none()
    {
        return Err(failure(
            path,
            OPERATION,
            io::ErrorKind::InvalidInput,
            "A settings file must be an ordinary file.",
        ));
    }
    let length = entry
        .item
        .metadata
        .size
        .filter(|length| *length <= MAX_PREFERENCES_BYTES as u64)
        .ok_or_else(|| {
            failure(
                path,
                OPERATION,
                io::ErrorKind::InvalidData,
                "The file is larger than the 4 MiB settings limit.",
            )
        })?;
    let revalidate = |source: &dyn TransferSource| {
        with_io(path, OPERATION, cancel, || {
            directory.revalidate(cancel)?;
            source.revalidate(cancel)
        })
    };
    let mut bytes = Vec::with_capacity(length as usize);
    let mut buffer = vec![0; READ_CHUNK];
    revalidate(source.as_ref())?;
    loop {
        let count = match with_io(path, OPERATION, cancel, || {
            source.read_chunk(&mut buffer, cancel)
        }) {
            Err(error) if error.kind == io::ErrorKind::Interrupted && !cancel.is_cancelled() => {
                continue;
            }
            result => result?,
        };
        if count > buffer.len() || count > (length as usize).saturating_sub(bytes.len()) {
            return Err(changed(path));
        }
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    revalidate(source.as_ref())?;
    if bytes.len() as u64 != length {
        return Err(changed(path));
    }
    decode_settings_file(&bytes).map_err(|error| FsError::from_io(OPERATION, path, error))
}

/// The whole snapshot as an in-memory transfer source for verified publication.
struct SettingsBytes {
    entry: TransferEntry,
    bytes: Arc<[u8]>,
    offset: usize,
}

impl TransferSource for SettingsBytes {
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }
    fn native_identity(&self) -> Option<TransferIdentity> {
        None
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        if cancel.is_cancelled() {
            Err(cancelled(&self.entry.item.path, "export settings"))
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
            "export settings",
            io::ErrorKind::Unsupported,
            "A settings file is not a folder.",
        ))
    }
}

/// An absolute native file path without `.`/`..` steps or NUL, split into its
/// pinned parent and final name. Display text never becomes a path here.
fn split<'a>(path: &'a Path, operation: &'static str) -> Result<(&'a Path, &'a OsStr), FsError> {
    let invalid = |message| failure(path, operation, io::ErrorKind::InvalidInput, message);
    if !path.is_absolute()
        || path.as_os_str().as_encoded_bytes().contains(&0)
        || path
            .components()
            .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
    {
        return Err(invalid(
            "Choose an absolute settings file path without parent steps or NUL.",
        ));
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => Ok((parent, name)),
        _ => Err(invalid("The settings file path must include a file name.")),
    }
}

fn with_io<T>(
    path: &Path,
    operation: &'static str,
    cancel: &Cancellation,
    work: impl FnOnce() -> Result<T, FsError>,
) -> Result<T, FsError> {
    let _permit = midnight_work_budget::shared_io()
        .acquire(IO, || cancel.is_cancelled())
        .ok_or_else(|| cancelled(path, operation))?;
    work()
}

fn changed(path: &Path) -> FsError {
    failure(
        path,
        "import settings",
        io::ErrorKind::InvalidData,
        "The settings file changed while it was being read.",
    )
}

fn cancelled(path: &Path, operation: &'static str) -> FsError {
    failure(path, operation, io::ErrorKind::Interrupted, "Cancelled.")
}

fn worker_failure(path: &Path) -> FsError {
    failure(
        path,
        "export settings",
        io::ErrorKind::Other,
        "The settings export worker failed.",
    )
}

fn failure(path: &Path, operation: &'static str, kind: io::ErrorKind, message: &str) -> FsError {
    FsError::from_io(operation, path, io::Error::new(kind, message))
}

#[cfg(test)]
#[path = "preferences_file_tests.rs"]
mod tests;
