//! Retry admission without replacing native objects already pinned by an earlier attempt.
//! Construct, attempt and drop only on backend workers. Successful admission releases
//! every cache owner immediately; the Job alone owns its source/capture lifetimes.
use super::*;
use input::Resolved;
use std::{ffi::OsStr, path::PathBuf, sync::MutexGuard};
#[cfg(test)]
#[path = "copy_preparation_handoff_tests.rs"]
mod handoff_tests;
#[path = "copy_planned_source.rs"]
mod planned_source;
#[cfg(test)]
#[path = "copy_planning_tests.rs"]
mod planning_tests;

type EndpointPins = Vec<(
    TransferNamespaceAccess,
    Option<Arc<dyn TransferNamespacePin>>,
)>;

pub(super) struct Preparation {
    // Failed attempts retain native pins. Destroy those authorities while input
    // still owns the captured provider registration and capture permit.
    service: RetainedService,
    planned: Option<PlannedInput>,
    endpoints: Option<EndpointPins>,
    destination_pin: Option<PinnedDirectory>,
    input: CopyInput,
    finished: bool,
    tracker: Option<Arc<Mutex<Tracker>>>,
    audited_native_children: bool,
}

impl Preparation {
    #[cfg(test)]
    pub(super) fn new(input: CopyInput, service: Arc<dyn TransferService>) -> Self {
        Self::qualified(
            input,
            crate::transfer_composition::QualifiedTransferInstallation::unqualified(service),
        )
    }
    pub(super) fn qualified(
        input: CopyInput,
        service: crate::transfer_composition::QualifiedTransferInstallation,
    ) -> Self {
        let mut input = ownership::Armed::new(input);
        let audited_native_children = input.native_children_are_audited();
        Self {
            service: RetainedService::qualified(service),
            planned: None,
            endpoints: None,
            destination_pin: None,
            input: input.into_inner(),
            finished: false,
            tracker: None,
            audited_native_children,
        }
    }

    pub(super) fn retirement_is_checked(&self) -> bool {
        self.service.witness.is_some() && self.audited_native_children
    }

    pub(super) fn attach_tracker(&mut self, tracker: Arc<Mutex<Tracker>>) {
        self.tracker = Some(tracker);
    }

    /// Whether `path` is one of several planned native roots, which Skip can leave out
    /// before anything is copied. Captured and archive sources stay whole.
    pub(super) fn skippable_root(&self, path: &Path) -> bool {
        matches!(&self.planned, Some(PlannedInput::Native(request, _))
            if request.clipboard.paths.len() > 1
                && request.clipboard.paths.iter().any(|root| root == path))
    }

    /// Skip: plan the job again without the root at `path`. A namespace plan that is
    /// already installed keeps claiming it, which only waits for more than needed.
    pub(super) fn skip_root(&mut self, path: &Path) {
        if let Some(PlannedInput::Native(request, _)) = &mut self.planned {
            request.clipboard.paths.retain(|root| root != path);
        }
        self.endpoints = None;
    }

    /// Read-only endpoint capture. The caller must grant the complete union before
    /// attempt; an absent capability is an opaque endpoint, never an empty claim.
    pub(super) fn pin_endpoints(
        &mut self,
        cancel: &Cancellation,
        status: &Status,
    ) -> Result<EndpointPins, FsError> {
        check_cancel(&self.input.destination, cancel)?;
        self.input.validate()?;
        if !self.retirement_is_checked() {
            return Err(error(
                &self.input.destination,
                io::ErrorKind::Unsupported,
                "Transfer sources do not supply an audited native composition and consuming provider release.",
            ));
        }
        if self.finished {
            return Err(error(
                &self.input.destination,
                io::ErrorKind::InvalidInput,
                "Transfer preparation has already completed.",
            ));
        }
        if let Some(endpoints) = &self.endpoints {
            for (_, pin) in endpoints {
                if let Some(pin) = pin {
                    pin.refresh(cancel)?;
                }
            }
            return Ok(endpoints.clone());
        }
        if self.planned.is_none() {
            let resolved = self.input.clone().resolve(&self.service, cancel)?;
            self.planned = Some(PlannedInput::new(resolved));
        }
        // Retain the original existing destination before opening a write-capable
        // destination. Unsupported read-only providers take the opaque lane.
        if self.destination_pin.is_none() {
            match self
                .service
                .service
                .prepare_checksum_directory(&self.input.destination, cancel)
            {
                Ok(directory) => {
                    let pin = PinnedDirectory {
                        path: self.input.destination.clone(),
                        identity: directory.identity(),
                        directory,
                    };
                    pin.revalidate(cancel)?;
                    self.destination_pin = Some(pin);
                }
                Err(error) if error.kind == io::ErrorKind::Unsupported => {}
                // Native rename does not require directory enumeration rights.
                // A denied read-only probe cannot grant positive geometry, but
                // the opaque lane can still try metadata-only Move after grant.
                Err(error)
                    if self.input.intent == TransferIntent::Move
                        && error.kind == io::ErrorKind::PermissionDenied => {}
                Err(error) => return Err(error),
            }
        }
        let links = input::source_link_policy(self.input.links, self.input.metadata);
        let roots =
            self.planned
                .as_ref()
                .expect("planned input")
                .roots(&self.service, links, cancel)?;
        status
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .requested_roots = roots.len() as u64;
        let access = if self.input.intent == TransferIntent::Move {
            TransferNamespaceAccess::Write
        } else {
            TransferNamespaceAccess::Read
        };
        let mut endpoints = Vec::new();
        for root in roots {
            root.revalidate(cancel)?;
            // A followed directory claims its own folder like any source. A nested link
            // can lead outside it; such reads are not protected against another job's
            // writes there (only reads: a followed source is never changed).
            let pin = root.native_namespace_scope(cancel)?;
            endpoints.push((access, pin));
            let name = root.entry().item.path.file_name().ok_or_else(|| {
                error(
                    &root.entry().item.path,
                    io::ErrorKind::InvalidInput,
                    "Transfer source needs a basename.",
                )
            })?;
            let pin = match &self.destination_pin {
                Some(destination) => destination.directory.native_child_namespace_scope(
                    name,
                    root.entry().item.kind,
                    cancel,
                )?,
                None => None,
            };
            endpoints.push((TransferNamespaceAccess::Write, pin));
        }
        self.endpoints = Some(endpoints.clone());
        Ok(endpoints)
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
        // Refuse incompatible options before revalidating or preparing providers.
        self.input.validate()?;
        self.service.revalidate(&cancel)?;
        if let Some(destination) = &self.destination_pin {
            destination.revalidate(&cancel)?;
            // Preserve the planned existing root identity across the capability upgrade.
            let authority = if self.input.intent == TransferIntent::Move {
                Authority::Move
            } else {
                Authority::Copy(CopyLinkPolicy::Reject)
            };
            let writable = self
                .service
                .destination(&self.input.destination, authority, &cancel)?;
            if writable.identity() != destination.identity {
                return Err(error(
                    &self.input.destination,
                    io::ErrorKind::InvalidData,
                    "Planned transfer destination changed identity.",
                ));
            }
        }
        let metadata = self.input.metadata;
        let links = self.input.links;
        let resolved = match &self.planned {
            Some(planned) => planned.resolve(),
            None => self.input.clone().resolve(&self.service, &cancel)?,
        };
        let requested_roots = match &resolved {
            Resolved::Native(request) | Resolved::NativeMove(request) => {
                request.clipboard.paths.len()
            }
            Resolved::Archive { sources, .. } => sources.len(),
            Resolved::Captured { sources, .. } => match sources {
                PreparedCapturedTransfer::Copy(roots) => roots.len(),
                PreparedCapturedTransfer::Move(roots) => roots.len(),
                PreparedCapturedTransfer::MoveWithDirectoryLinks(roots) => roots.len(),
            },
        };
        status
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .requested_roots = requested_roots as u64;
        let mut job = ownership::Armed::new(match resolved {
            Resolved::NativeMove(request) => worker::Job::prepare_move_with_options(
                request,
                &self.service,
                cancel,
                status,
                metadata,
                links,
            ),
            Resolved::Native(request) => worker::Job::prepare_with_options(
                request,
                &self.service,
                cancel,
                status,
                metadata,
                links,
            ),
            Resolved::Captured {
                destination,
                sources: PreparedCapturedTransfer::Copy(sources),
            } => worker::Job::prepare_sources_with_options(
                destination,
                sources,
                &self.service,
                cancel,
                status,
                metadata,
                links,
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
            Resolved::Captured {
                destination,
                sources: PreparedCapturedTransfer::MoveWithDirectoryLinks(sources),
            } => worker::Job::prepare_move_entries(
                destination,
                sources,
                &self.service,
                cancel,
                status,
                metadata,
                links,
            ),
            Resolved::Archive {
                destination,
                sources,
            } => worker::Job::prepare_archive(destination, sources, &self.service, cancel, status),
        }?);
        if let Some(witness) = self.service.witness {
            job.set_retirement_witness(witness);
        }
        if let Some(locale) = &self.service.locale {
            job.set_locale(locale.clone());
        }
        if let Some(tracker) = &self.tracker {
            job.attach_tracker(tracker.clone());
        }
        job.set_verification(self.input.verification);
        job.prepare_discovery();
        if let Some(completions) = &self.input._completions {
            job.set_drop_completions(completions.clone());
        }
        self.finished = true;
        // In particular, do not keep the Move source or capture alive until the
        // whole job ends: all source owners must close before stage cleanup.
        self.service.release();
        self.planned = None;
        self.destination_pin = None;
        self.endpoints = None;
        Ok(job.into_inner())
    }
}

enum PlannedInput {
    Native(CopyRequest, bool),
    Copy(PathBuf, Vec<SharedSource>, bool),
    Move(PathBuf, Vec<(SharedSource, Arc<dyn TransferMoveCapture>)>),
    MoveWithDirectoryLinks(PathBuf, Vec<PlannedMoveEntry>),
}

enum PlannedMoveEntry {
    Ordinary(SharedSource, Arc<dyn TransferMoveCapture>),
    DirectoryLink(SharedSource, Arc<dyn TransferDirectoryLinkMoveCapture>),
}

impl PlannedMoveEntry {
    fn new(entry: PreparedTransferMoveEntry) -> Self {
        match entry {
            PreparedTransferMoveEntry::Ordinary(prepared) => {
                Self::Ordinary(SharedSource::new(prepared.source), prepared.capture)
            }
            PreparedTransferMoveEntry::DirectoryLink { source, capture } => {
                Self::DirectoryLink(SharedSource::new(source), capture)
            }
        }
    }

    fn source(&self) -> SharedSource {
        match self {
            Self::Ordinary(source, _) | Self::DirectoryLink(source, _) => source.clone(),
        }
    }

    fn prepared(&self) -> PreparedTransferMoveEntry {
        match self {
            Self::Ordinary(source, capture) => {
                PreparedTransferMoveEntry::Ordinary(PreparedTransferMove {
                    source: Box::new(source.clone()),
                    capture: capture.clone(),
                })
            }
            Self::DirectoryLink(source, capture) => PreparedTransferMoveEntry::DirectoryLink {
                source: Box::new(source.clone()),
                capture: capture.clone(),
            },
        }
    }
}

impl PlannedInput {
    fn new(resolved: Resolved) -> Self {
        match resolved {
            Resolved::Native(request) => Self::Native(request, false),
            Resolved::NativeMove(request) => Self::Native(request, true),
            Resolved::Archive {
                destination,
                sources,
            } => Self::Copy(
                destination,
                sources.into_iter().map(SharedSource::new).collect(),
                true,
            ),
            Resolved::Captured {
                destination,
                sources: PreparedCapturedTransfer::Copy(sources),
            } => Self::Copy(
                destination,
                sources.into_iter().map(SharedSource::new).collect(),
                false,
            ),
            Resolved::Captured {
                destination,
                sources: PreparedCapturedTransfer::Move(sources),
            } => Self::Move(
                destination,
                sources
                    .into_iter()
                    .map(|source| (SharedSource::new(source.source), source.capture))
                    .collect(),
            ),
            Resolved::Captured {
                destination,
                sources: PreparedCapturedTransfer::MoveWithDirectoryLinks(sources),
            } => Self::MoveWithDirectoryLinks(
                destination,
                sources.into_iter().map(PlannedMoveEntry::new).collect(),
            ),
        }
    }
    fn resolve(&self) -> Resolved {
        match self {
            Self::Native(request, moved) => {
                if *moved {
                    Resolved::NativeMove(request.clone())
                } else {
                    Resolved::Native(request.clone())
                }
            }
            Self::Copy(destination, roots, archive) => {
                let sources = roots
                    .iter()
                    .map(|source| Box::new(source.clone()) as Box<dyn TransferSource>)
                    .collect();
                if *archive {
                    Resolved::Archive {
                        destination: destination.clone(),
                        sources,
                    }
                } else {
                    Resolved::Captured {
                        destination: destination.clone(),
                        sources: PreparedCapturedTransfer::Copy(sources),
                    }
                }
            }
            Self::Move(destination, roots) => Resolved::Captured {
                destination: destination.clone(),
                sources: PreparedCapturedTransfer::Move(
                    roots
                        .iter()
                        .map(|(source, capture)| PreparedTransferMove {
                            source: Box::new(source.clone()),
                            capture: capture.clone(),
                        })
                        .collect(),
                ),
            },
            Self::MoveWithDirectoryLinks(destination, roots) => Resolved::Captured {
                destination: destination.clone(),
                sources: PreparedCapturedTransfer::MoveWithDirectoryLinks(
                    roots.iter().map(PlannedMoveEntry::prepared).collect(),
                ),
            },
        }
    }
    fn roots(
        &self,
        service: &RetainedService,
        links: CopyLinkPolicy,
        cancel: &Cancellation,
    ) -> Result<Vec<SharedSource>, FsError> {
        match self {
            Self::Native(request, moved) => request
                .clipboard
                .paths
                .iter()
                .map(|path| {
                    service
                        .source(
                            path,
                            if *moved && links != CopyLinkPolicy::Reject {
                                Authority::MoveWithLinks(links)
                            } else if *moved {
                                Authority::Move
                            } else {
                                Authority::Copy(links)
                            },
                            cancel,
                        )
                        .map(|source| source.source)
                })
                .collect(),
            Self::Copy(_, roots, _) => Ok(roots.clone()),
            Self::Move(_, roots) => Ok(roots.iter().map(|(source, _)| source.clone()).collect()),
            Self::MoveWithDirectoryLinks(_, roots) => {
                Ok(roots.iter().map(PlannedMoveEntry::source).collect())
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Authority {
    Copy(CopyLinkPolicy),
    Move,
    MoveWithLinks(CopyLinkPolicy),
}

#[derive(Default)]
struct Pins {
    sources: HashMap<(PathBuf, Authority), PinnedSource>,
    destinations: HashMap<(PathBuf, Authority), PinnedDirectory>,
}

/// Copy and Move preparation only: the sources it prepares compare their pinned revisions
/// under `TRANSFER_REVISIONS`, whichever policy a caller asks for.
struct RetainedService {
    service: Arc<dyn TransferService>,
    witness: Option<crate::transfer_composition::TransferRetirementWitness>,
    locale: Option<crate::transfer_composition::QualifiedLocaleInstallation>,
    pins: Mutex<Pins>,
}

impl RetainedService {
    #[cfg(test)]
    fn new(service: Arc<dyn TransferService>) -> Self {
        Self::qualified(
            crate::transfer_composition::QualifiedTransferInstallation::unqualified(service),
        )
    }
    fn qualified(service: crate::transfer_composition::QualifiedTransferInstallation) -> Self {
        Self {
            witness: service.witness(),
            locale: service.locale(),
            service: service.service(),
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
            let (source, capture, link_capture) = match authority {
                Authority::Copy(links) => (
                    self.service.prepare_transfer_source_with_policy(
                        path,
                        links,
                        TRANSFER_REVISIONS,
                        cancel,
                    )?,
                    None,
                    None,
                ),
                Authority::Move => {
                    let prepared =
                        prepare_ordinary_move_source(self.service.as_ref(), path, cancel)?;
                    (prepared.source, Some(prepared.capture), None)
                }
                Authority::MoveWithLinks(links) => {
                    match self.service.prepare_move_source_with_policy(
                        path,
                        links,
                        TRANSFER_REVISIONS,
                        cancel,
                    )? {
                        PreparedTransferMoveEntry::Ordinary(prepared) => {
                            (prepared.source, Some(prepared.capture), None)
                        }
                        PreparedTransferMoveEntry::DirectoryLink { source, capture } => {
                            (source, None, Some(capture))
                        }
                    }
                }
            };
            // Install the original owner before any fallible pin qualification.
            // A failed probe must never permit Retry to capture a new pathname.
            let ready = PinnedSource {
                source: SharedSource::new(source),
                capture,
                link_capture,
            };
            let displaced = self.pins(path)?.sources.insert(key.clone(), ready.clone());
            drop(displaced);
            if ready.link_capture.is_some() {
                ready
            } else {
                let source = planned_source::defer(
                    self.service.clone(),
                    authority,
                    Box::new(ready.source.clone()),
                    ready.capture.clone(),
                    cancel,
                )?;
                let displaced = self.pins(path)?.sources.insert(key, source.clone());
                drop(displaced);
                source
            }
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
                Authority::Copy(_) => self.service.prepare_transfer_destination(path, cancel)?,
                Authority::Move | Authority::MoveWithLinks(_) => {
                    self.service.prepare_move_destination(path, cancel)?
                }
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
        self.prepare_transfer_source_with_links(path, CopyLinkPolicy::Reject, cancel)
    }

    fn prepare_transfer_source_with_links(
        &self,
        path: &Path,
        links: CopyLinkPolicy,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        Ok(Box::new(
            self.source(path, Authority::Copy(links), cancel)?.source,
        ))
    }

    fn prepare_transfer_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.destination(path, Authority::Copy(CopyLinkPolicy::Reject), cancel)
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

    fn prepare_move_source_with_links(
        &self,
        path: &Path,
        links: CopyLinkPolicy,
        cancel: &Cancellation,
    ) -> Result<PreparedTransferMoveEntry, FsError> {
        if links == CopyLinkPolicy::Reject {
            return self
                .prepare_move_source(path, cancel)
                .map(PreparedTransferMoveEntry::Ordinary);
        }
        let pinned = self.source(path, Authority::MoveWithLinks(links), cancel)?;
        let source = Box::new(pinned.source);
        match (pinned.capture, pinned.link_capture) {
            (Some(capture), None) => {
                Ok(PreparedTransferMoveEntry::Ordinary(PreparedTransferMove {
                    source,
                    capture,
                }))
            }
            (None, Some(capture)) => {
                Ok(PreparedTransferMoveEntry::DirectoryLink { source, capture })
            }
            _ => Err(error(
                path,
                io::ErrorKind::InvalidData,
                "Retained Move source does not carry exactly one mutation authority.",
            )),
        }
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
    link_capture: Option<Arc<dyn TransferDirectoryLinkMoveCapture>>,
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
        if let Some(capture) = &self.link_capture {
            capture.revalidate(cancel)?;
            let original = capture.entry();
            if original.item.path != self.source.entry.item.path
                || original.item.kind != EntryKind::DirectoryLink
                || Some(original.identity) != self.source.native_original_identity()
            {
                return Err(error(
                    &self.source.entry.item.path,
                    io::ErrorKind::InvalidData,
                    "Retained directory projection and original link no longer match.",
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
    native_original_identity: Option<TransferIdentity>,
    virtual_identity: Option<VirtualEntryId>,
    source: Arc<Mutex<Box<dyn TransferSource>>>,
}

impl SharedSource {
    fn new(source: Box<dyn TransferSource>) -> Self {
        Self {
            entry: source.entry().clone(),
            native_identity: source.native_identity(),
            native_original_identity: source.native_original_identity(),
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
    fn native_namespace_scope(
        &self,
        cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn TransferNamespacePin>>, FsError> {
        self.source()?.native_namespace_scope(cancel)
    }
    fn discover(
        &self,
        cancel: &Cancellation,
        visit: &mut dyn FnMut(&MutationItem),
    ) -> Result<(), FsError> {
        self.revalidate(cancel)?;
        self.source()?.discover(cancel, visit)
    }
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }

    fn native_identity(&self) -> Option<TransferIdentity> {
        self.native_identity
    }

    fn native_original_identity(&self) -> Option<TransferIdentity> {
        self.native_original_identity
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
            || source.native_original_identity() != self.native_original_identity
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

    fn link_representation(
        &self,
        cancel: &Cancellation,
    ) -> Result<TransferLinkRepresentation, FsError> {
        self.revalidate(cancel)?;
        self.source()?.link_representation(cancel)
    }

    fn link_not_followed(&self) -> Option<String> {
        self.source().ok()?.link_not_followed()
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

    fn next_move_child_with_links(
        &mut self,
        links: CopyLinkPolicy,
        cancel: &Cancellation,
    ) -> Result<Option<PreparedTransferMoveEntry>, FsError> {
        self.source()?.next_move_child_with_links(links, cancel)
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

    fn next_child_or_failure(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<TransferChild>, FsError> {
        self.source()?.next_child_or_failure(cancel)
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
