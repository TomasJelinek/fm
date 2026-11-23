//! How a Job is built from prepared roots: the constructors that check and admit them.
use super::*;

impl Job {
    pub(in crate::copy_operations) fn prepare_move_sources(
        destination: std::path::PathBuf,
        roots: VecDeque<PreparedTransferMove>,
        service: &dyn TransferService,
        cancel: Cancellation,
        status: Status,
        metadata: TransferMetadataPolicy,
    ) -> Result<Self, FsError> {
        move_job::prepare_sources(destination, roots, service, cancel, status, metadata)
    }

    pub(in crate::copy_operations) fn prepare_move_entries(
        destination: std::path::PathBuf,
        roots: VecDeque<PreparedTransferMoveEntry>,
        service: &dyn TransferService,
        cancel: Cancellation,
        status: Status,
        metadata: TransferMetadataPolicy,
        links: CopyLinkPolicy,
    ) -> Result<Self, FsError> {
        move_job::prepare_entries(destination, roots, service, cancel, status, metadata, links)
    }

    pub(in crate::copy_operations) fn prepare_with_metadata(
        request: CopyRequest,
        service: &dyn TransferService,
        cancel: Cancellation,
        status: Status,
        metadata: TransferMetadataPolicy,
    ) -> Result<Self, FsError> {
        if metadata == TransferMetadataPolicy::ContentOnly {
            return Self::prepare(request, service, cancel, status);
        }
        move_job::check_bounds(&request)?;
        let mut job = ownership::Armed::new(Self::prepare(request, service, cancel, status)?);
        let permit = match move_job::permit(&job.cancel, &job.progress.destination_directory) {
            Ok(permit) => permit,
            Err(error) => {
                drop(job.into_inner());
                return Err(error);
            }
        };
        if let Err(error) = job.prepare_metadata(metadata) {
            drop(job.into_inner());
            return Err(error);
        }
        drop(permit);
        Ok(job.into_inner())
    }
    pub(in crate::copy_operations) fn prepare_move_with_metadata(
        request: CopyRequest,
        service: &dyn TransferService,
        cancel: Cancellation,
        status: Status,
        metadata: TransferMetadataPolicy,
    ) -> Result<Self, FsError> {
        if metadata == TransferMetadataPolicy::ContentOnly {
            return Self::prepare_move(request, service, cancel, status);
        }
        move_job::prepare_with_metadata(request, service, cancel, status, metadata)
    }
    pub(in crate::copy_operations) fn prepare_move_with_options(
        request: CopyRequest,
        service: &dyn TransferService,
        cancel: Cancellation,
        status: Status,
        metadata: TransferMetadataPolicy,
        links: CopyLinkPolicy,
    ) -> Result<Self, FsError> {
        if links == CopyLinkPolicy::Reject {
            return Self::prepare_move_with_metadata(request, service, cancel, status, metadata);
        }
        move_job::prepare_with_options(request, service, cancel, status, metadata, links)
    }

    pub(in crate::copy_operations) fn prepare_move(
        request: CopyRequest,
        service: &dyn TransferService,
        cancel: Cancellation,
        status: Status,
    ) -> Result<Self, FsError> {
        move_job::prepare(request, service, cancel, status)
    }
    pub(in crate::copy_operations) fn prepare(
        request: CopyRequest,
        service: &dyn TransferService,
        cancel: Cancellation,
        status: Status,
    ) -> Result<Self, FsError> {
        Self::prepare_native(request, service, cancel, status, CopyLinkPolicy::Reject)
    }

    pub(in crate::copy_operations) fn prepare_with_options(
        request: CopyRequest,
        service: &dyn TransferService,
        cancel: Cancellation,
        status: Status,
        metadata: TransferMetadataPolicy,
        links: CopyLinkPolicy,
    ) -> Result<Self, FsError> {
        let links = input::source_link_policy(links, metadata);
        if links == CopyLinkPolicy::Reject {
            return Self::prepare_with_metadata(request, service, cancel, status, metadata);
        }
        link_job::validate_options(&request.destination, &status, metadata, links)?;
        Self::prepare_native(request, service, cancel, status, links)
    }

    pub(in crate::copy_operations::worker) fn prepare_native(
        request: CopyRequest,
        service: &dyn TransferService,
        cancel: Cancellation,
        status: Status,
        links: CopyLinkPolicy,
    ) -> Result<Self, FsError> {
        let destination = service.prepare_transfer_destination(&request.destination, &cancel)?;
        if destination.path() != request.destination {
            return Err(error(
                &request.destination,
                io::ErrorKind::InvalidInput,
                "Provider returned a different destination.",
            ));
        }
        let mut seen = HashSet::new();
        let mut roots = VecDeque::new();
        for path in &request.clipboard.paths {
            if cancel.is_cancelled() {
                return Err(error(path, io::ErrorKind::Interrupted, "Copy cancelled."));
            }
            if !seen.insert(path) {
                continue;
            }
            let source = service.prepare_transfer_source_with_policy(
                path,
                links,
                TRANSFER_REVISIONS,
                &cancel,
            )?;
            if source.entry().item.path != *path {
                return Err(error(
                    path,
                    io::ErrorKind::InvalidInput,
                    "Provider returned a different source.",
                ));
            }
            if source.entry().item.kind == EntryKind::Directory {
                destination.reject_source_ancestor(source.entry().identity, &cancel)?;
            }
            roots.push_back(source);
        }
        link_job::admit_roots(&roots, links, &cancel)?;
        let mut job = ownership::Armed::new(Self::from_sources(destination, roots, cancel, status));
        job.links = links;
        job.set_verification(request.verification);
        Ok(job.into_inner())
    }

    pub(in crate::copy_operations) fn prepare_archive(
        path: std::path::PathBuf,
        roots: VecDeque<Box<dyn TransferSource>>,
        service: &dyn TransferService,
        cancel: Cancellation,
        status: Status,
    ) -> Result<Self, FsError> {
        let destination = service.prepare_transfer_destination(&path, &cancel)?;
        if destination.path() != path {
            return Err(error(
                &path,
                io::ErrorKind::InvalidInput,
                "Provider returned a different destination.",
            ));
        }
        Ok(Self::from_sources(destination, roots, cancel, status))
    }

    pub(in crate::copy_operations) fn prepare_sources(
        path: std::path::PathBuf,
        roots: VecDeque<Box<dyn TransferSource>>,
        service: &dyn TransferService,
        cancel: Cancellation,
        status: Status,
        metadata: TransferMetadataPolicy,
    ) -> Result<Self, FsError> {
        Self::prepare_captured(
            path,
            roots,
            service,
            cancel,
            status,
            metadata,
            CopyLinkPolicy::Reject,
        )
    }

    pub(in crate::copy_operations) fn prepare_sources_with_options(
        path: std::path::PathBuf,
        roots: VecDeque<Box<dyn TransferSource>>,
        service: &dyn TransferService,
        cancel: Cancellation,
        status: Status,
        metadata: TransferMetadataPolicy,
        links: CopyLinkPolicy,
    ) -> Result<Self, FsError> {
        let links = input::source_link_policy(links, metadata);
        if links == CopyLinkPolicy::Reject {
            return Self::prepare_sources(path, roots, service, cancel, status, metadata);
        }
        link_job::validate_options(&path, &status, metadata, links)?;
        Self::prepare_captured(path, roots, service, cancel, status, metadata, links)
    }

    pub(in crate::copy_operations::worker) fn prepare_captured(
        path: std::path::PathBuf,
        roots: VecDeque<Box<dyn TransferSource>>,
        service: &dyn TransferService,
        cancel: Cancellation,
        status: Status,
        metadata: TransferMetadataPolicy,
        links: CopyLinkPolicy,
    ) -> Result<Self, FsError> {
        // Link-aware callers validate their options before entering preparation.
        if roots.is_empty() {
            return Err(error(
                &path,
                io::ErrorKind::InvalidInput,
                "Invalid captured Copy root count.",
            ));
        }
        // Only native roots hold handles open until copied; archive members take any count.
        super::input::check_native_roots(
            &path,
            roots
                .iter()
                .filter(|source| source.native_identity().is_some())
                .count(),
        )?;
        if metadata != TransferMetadataPolicy::ContentOnly {
            let paths = roots
                .iter()
                .map(|source| source.entry().item.path.clone())
                .collect();
            move_job::check_bounds(&CopyRequest {
                start_policy: TransferStartPolicy::Immediate,
                verification: snapshot(&status).verification,
                id: snapshot(&status).id,
                clipboard: FileClipboard {
                    paths,
                    intent: FileClipboardIntent::Copy,
                },
                destination: path.clone(),
            })?;
        }
        let destination = service.prepare_transfer_destination(&path, &cancel)?;
        if destination.path() != path {
            return Err(error(
                &path,
                io::ErrorKind::InvalidInput,
                "Provider returned a different destination.",
            ));
        }
        let mut identities = HashSet::new();
        for source in &roots {
            source.revalidate(&cancel)?;
            if !matches!(
                source.entry().item.kind,
                EntryKind::File | EntryKind::Directory
            ) && !(links.recreates_links() && link_job::is_link_kind(source.entry().item.kind))
            {
                return Err(error(
                    &path,
                    io::ErrorKind::Unsupported,
                    "Captured Copy source kind is unsupported by this link policy.",
                ));
            }
            if let Some(id) = source.native_identity() {
                if id != source.entry().identity {
                    return Err(error(
                        &path,
                        io::ErrorKind::InvalidData,
                        "Native Copy source identity is inconsistent.",
                    ));
                }
                let original = if links.materializes_directory_links() {
                    source.native_original_identity().ok_or_else(|| {
                        error(
                            &source.entry().item.path,
                            io::ErrorKind::InvalidData,
                            "Native Copy is missing the original source identity.",
                        )
                    })?
                } else {
                    id
                };
                if !identities.insert(original) {
                    return Err(error(
                        &path,
                        io::ErrorKind::InvalidInput,
                        "Duplicate native Copy roots.",
                    ));
                }
                if source.entry().item.kind == EntryKind::Directory {
                    destination.reject_source_ancestor(id, &cancel)?;
                }
            }
        }
        // Read-only ancestry authority avoids requiring writes to source parents.
        let directories: Vec<_> = roots
            .iter()
            .filter(|source| source.entry().item.kind == EntryKind::Directory)
            .filter_map(|source| source.native_identity())
            .collect();
        let mut parents = Vec::new();
        if !directories.is_empty() {
            // Roots of one selection usually share a parent: check each distinct parent
            // once (O(distinct parents x directory roots)), not once per root.
            let mut checked = HashSet::new();
            for source in roots
                .iter()
                .filter(|source| source.native_identity().is_some())
            {
                let source_path = &source.entry().item.path;
                let parent = source_path.parent().ok_or_else(|| {
                    error(
                        source_path,
                        io::ErrorKind::InvalidInput,
                        "Copy source needs a parent.",
                    )
                })?;
                if !checked.insert(parent.to_path_buf()) {
                    continue;
                }
                let directory = service.prepare_checksum_directory(parent, &cancel)?;
                if directory.path() != parent {
                    return Err(error(
                        parent,
                        io::ErrorKind::InvalidData,
                        "Provider returned a different source parent.",
                    ));
                }
                for id in &directories {
                    directory.reject_source_ancestor(*id, &cancel)?;
                }
                parents.push(directory);
            }
        }
        for source in &roots {
            source.revalidate(&cancel)?;
        }
        link_job::admit_roots(&roots, links, &cancel)?;
        let mut job = ownership::Armed::new(Self::from_sources(destination, roots, cancel, status));
        job.links = links;
        if metadata != TransferMetadataPolicy::ContentOnly {
            // Input resolution refuses more roots before any capture (`copy_input`); the
            // job keeps its own guard, in the same words.
            if job.roots.len() > midnight_core::MAX_REQUIRED_METADATA_ROOTS {
                let error = error(
                    &path,
                    io::ErrorKind::Unsupported,
                    midnight_core::REQUIRED_METADATA_ROOTS_REFUSAL,
                );
                drop(job.into_inner());
                return Err(error);
            }
            if let Err(error) = job.prepare_metadata(metadata) {
                drop(job.into_inner());
                return Err(error);
            }
        }
        // A provider-backed parent destructor must run before the Job leaves
        // armed storage, even after successful metadata preflight.
        drop(parents);
        Ok(job.into_inner())
    }

    pub(in crate::copy_operations::worker) fn from_sources(
        destination: Arc<dyn TransferDirectory>,
        roots: VecDeque<Box<dyn TransferSource>>,
        cancel: Cancellation,
        status: Status,
    ) -> Self {
        let protected_roots = roots
            .iter()
            .filter_map(|source| source.native_identity())
            .collect();
        let mut progress = snapshot(&status);
        progress.requested_roots = roots.len() as u64;
        let link_retirement = Arc::new(link_job::LinkRetirement::new());
        Self {
            retirement_witness: None,
            locale: None,
            progress,
            tracker: Arc::new(Mutex::new(Tracker::default())),
            pass_ids: crate::operation_throughput::PassIds::default(),
            finish_started: false,
            namespace_busy: None,
            final_name_collision: false,
            status,
            cancel,
            destination,
            roots,
            unstarted: None,
            streamed: None,
            awaiting_identities: None,
            protected_roots,
            stack: vec![],
            failed_child: None,
            reopened_child: None,
            reprepared_child: None,
            current: None,
            conflict: None,
            failure: None,
            failure_context: failures::Context::Entry,
            committed: None,
            skipped_errors: HashSet::new(),
            halt: false,
            decision: None,
            version: 0,
            skip_all: false,
            replace_all: false,
            merge_all: false,
            move_work: None,
            metadata: None,
            verification: TransferVerification::default(),
            links: CopyLinkPolicy::Reject,
            work_budget: work_budget::Budget::default(),
            active_root: None,
            completed_root: None,
            drop_retention: None,
            drop_completions: None,
            link_retirement,
        }
    }
}
