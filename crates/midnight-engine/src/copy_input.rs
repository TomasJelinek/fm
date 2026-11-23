//! One admission path for paste, drops and immutable panel selections.
use super::*;
use std::path::PathBuf;

#[cfg(test)]
#[path = "copy_root_bound_tests.rs"]
mod root_bound_tests;
#[cfg(test)]
#[path = "copy_start_policy_tests.rs"]
mod start_policy_tests;

#[derive(Clone)]
enum Sources {
    Paths(Arc<FileClipboard>),
    SharedPaths(Arc<FileClipboard>),
    Selection(SelectedFiles),
    Archive(ArchiveOwner),
    Captured(CapturedOwner),
}

#[derive(Clone)]
struct CapturedOwner(Option<Arc<dyn CapturedTransferSources>>);
impl std::ops::Deref for CapturedOwner {
    type Target = dyn CapturedTransferSources;
    fn deref(&self) -> &Self::Target {
        &**self.0.as_ref().expect("captured input remains owned")
    }
}
impl Drop for CapturedOwner {
    fn drop(&mut self) {
        if let Some(source) = self.0.take() {
            if crate::copy_nested_retirement::current().is_some() {
                let _ = crate::copy_nested_retirement::release_transfer_sources(source);
            } else {
                drop(source);
            }
        }
    }
}

#[derive(Clone)]
struct ArchiveOwner(Option<Box<ArchiveCopyRequest>>);
impl std::ops::Deref for ArchiveOwner {
    type Target = ArchiveCopyRequest;
    fn deref(&self) -> &Self::Target {
        self.0.as_ref().expect("archive input remains owned")
    }
}
impl Drop for ArchiveOwner {
    fn drop(&mut self) {
        let Some(mut request) = self.0.take() else {
            return;
        };
        if crate::copy_nested_retirement::current().is_some()
            && let Some(mount) = request.location.mount.take()
        {
            let _ = crate::copy_nested_retirement::release_archive(mount);
        }
        drop(request);
    }
}

impl From<CapturedTransferRequest> for CopyInput {
    fn from(request: CapturedTransferRequest) -> Self {
        Self {
            id: request.id,
            destination: request.destination,
            // The retained source is armed before its first foreign accessor.
            // The bounded worker initializes the actual intent before publishing.
            intent: TransferIntent::Copy,
            metadata: request.metadata,
            verification: request.verification,
            start_policy: request.start_policy,
            links: CopyLinkPolicy::Reject,
            _completions: None,
            audited_native_children: None,
            sources: Sources::Captured(CapturedOwner(Some(request.sources))),
        }
    }
}
impl From<ArchiveCopyRequest> for CopyInput {
    fn from(request: ArchiveCopyRequest) -> Self {
        Self {
            id: request.id,
            destination: request.destination.clone(),
            intent: TransferIntent::Copy,
            metadata: TransferMetadataPolicy::ContentOnly,
            verification: request.verification,
            start_policy: request.start_policy,
            links: CopyLinkPolicy::Reject,
            _completions: None,
            audited_native_children: None,
            sources: Sources::Archive(ArchiveOwner(Some(Box::new(request)))),
        }
    }
}

/// A native selection of more items than a job prepares before it starts
/// (`MAX_PREPARED_NATIVE_ROOTS`): the job keeps the paths, shared with its input, and
/// takes the items one at a time (`worker::streamed`).
#[derive(Clone)]
pub(super) struct StreamedRequest {
    pub(super) destination: PathBuf,
    pub(super) verification: TransferVerification,
    pub(super) paths: Arc<FileClipboard>,
    pub(super) moved: bool,
}

/// The most native items that a Copy or Move prepares before it starts: it opens and
/// pins each of them, and a failing one can be left out before anything is copied. A
/// larger selection streams instead (`StreamedRequest`).
pub(super) const MAX_PREPARED_NATIVE_ROOTS: usize = 64;

pub(super) enum Resolved {
    Native(CopyRequest),
    NativeMove(CopyRequest),
    Streamed(StreamedRequest),
    Captured {
        destination: PathBuf,
        sources: PreparedCapturedTransfer,
    },
    Archive {
        destination: PathBuf,
        sources: std::collections::VecDeque<Box<dyn TransferSource>>,
    },
}

#[derive(Clone)]
pub(super) struct CopyInput {
    pub(super) id: OperationId,
    pub(super) destination: PathBuf,
    pub(super) intent: TransferIntent,
    pub(super) metadata: TransferMetadataPolicy,
    pub(super) verification: TransferVerification,
    pub(super) start_policy: TransferStartPolicy,
    // Frozen at admission with the source capture and reused on every Retry.
    pub(super) links: CopyLinkPolicy,
    sources: Sources,
    // Preparation retains failed/retried admission and hands successful ownership
    // to Job. WorkerOwned keeps cancellation destruction off UI.
    pub(super) _completions: Option<Arc<HashMap<PathBuf, TreeCopyOccurrence>>>,
    audited_native_children: Option<bool>,
}

impl From<CopyRequest> for CopyInput {
    fn from(request: CopyRequest) -> Self {
        Self {
            id: request.id,
            destination: request.destination,
            intent: TransferIntent::Copy,
            metadata: TransferMetadataPolicy::ContentOnly,
            verification: request.verification,
            start_policy: request.start_policy,
            links: CopyLinkPolicy::Reject,
            _completions: None,
            audited_native_children: None,
            sources: Sources::Paths(Arc::new(request.clipboard)),
        }
    }
}

impl From<SelectedCopyRequest> for CopyInput {
    fn from(request: SelectedCopyRequest) -> Self {
        Self {
            id: request.id,
            destination: request.destination,
            intent: TransferIntent::Copy,
            metadata: TransferMetadataPolicy::ContentOnly,
            verification: request.verification,
            start_policy: request.start_policy,
            links: CopyLinkPolicy::Reject,
            _completions: None,
            audited_native_children: None,
            sources: Sources::Selection(request.files),
        }
    }
}

/// One Copy or Move takes at most `MAX_NATIVE_TRANSFER_ROOTS` native items: it keeps their
/// paths and a few bytes per item until each is copied. A file list of any length reaches
/// here (the clipboard, a drop, a panel selection).
pub(crate) fn check_native_roots(
    destination: &std::path::Path,
    count: usize,
) -> Result<(), FsError> {
    if count > MAX_NATIVE_TRANSFER_ROOTS {
        return Err(error(
            destination,
            io::ErrorKind::InvalidInput,
            &format!(
                "Copy and Move take at most {} items at once; these are {count}. Copy or move \
                 their folder instead, or fewer items at a time.",
                midnight_core::MAX_NATIVE_TRANSFER_ROOTS_TEXT
            ),
        ));
    }
    Ok(())
}

/// Required modified time pins and stages every root before anything is copied: the
/// backend takes at most `MAX_REQUIRED_METADATA_ROOTS` of them, whatever the dialog
/// offered, and refuses more before any source is captured, never as a content-only
/// transfer, in the policy's words for a Copy and a Move alike.
fn check_required_metadata_roots(
    destination: &std::path::Path,
    metadata: TransferMetadataPolicy,
    count: usize,
) -> Result<(), FsError> {
    if metadata == TransferMetadataPolicy::RequireModifiedTime
        && count > midnight_core::MAX_REQUIRED_METADATA_ROOTS
    {
        return Err(error(
            destination,
            io::ErrorKind::Unsupported,
            midnight_core::REQUIRED_METADATA_ROOTS_REFUSAL,
        ));
    }
    Ok(())
}

fn metadata_id(request: &MetadataTransferRequest) -> OperationId {
    match &request.input {
        MetadataTransferInput::Paths(request) => request.id,
        MetadataTransferInput::Selected(request) => request.id,
        MetadataTransferInput::Clipboard(request) => request.id,
    }
}
/// Exact modified-time preflight accepts only native regular files. It keeps
/// the ordinary no-follow source route even when the request carries a link
/// option, so links stay refused there as before.
pub(super) fn source_link_policy(
    links: CopyLinkPolicy,
    metadata: TransferMetadataPolicy,
) -> CopyLinkPolicy {
    if metadata == TransferMetadataPolicy::RequireModifiedTime {
        CopyLinkPolicy::Reject
    } else {
        links
    }
}

fn options_id(request: &CopyOptionsRequest) -> OperationId {
    match &request.input {
        CopyOptionsInput::Native(request) => metadata_id(request),
        CopyOptionsInput::Captured(request) => request.id,
    }
}

/// Generates both views of the one Copy-family table, so the borrowed classification
/// (guarded admission and the command router) and the consuming conversion
/// (`convert_submission`) can never disagree about a variant.
macro_rules! copy_family {
    ($($variant:ident($request:ident) => $id:expr, $input:expr;)+) => {
        /// The operation ID of a Copy-family command (Copy, Move, metadata, options,
        /// archive and captured transfers); None for every other command.
        pub(crate) fn copy_command_id(command: &Command) -> Option<OperationId> {
            match command {
                $(Command::$variant($request) => Some($id),)+
                _ => None,
            }
        }

        impl CopyInput {
            #[expect(
                clippy::result_large_err,
                reason = "Return unmatched commands to their armed admission scope without an extra box."
            )]
            pub(super) fn from_command(command: Command) -> Result<Self, Command> {
                match command {
                    $(Command::$variant($request) => Ok($input),)+
                    command => Err(command),
                }
            }
        }
    };
}
copy_family! {
    EnqueueCopy(request) => request.id, request.into();
    EnqueueSelectedCopy(request) => request.id, request.into();
    EnqueueMove(request) => request.id, Self::move_paths(request);
    EnqueueSelectedMove(request) => request.id, Self::move_selection(request);
    EnqueueClipboardMove(request) => request.id, Self::move_clipboard(request);
    EnqueueTransferWithMetadata(request) => metadata_id(request), Self::metadata(*request);
    EnqueueCopyWithOptions(request) => options_id(request), Self::options(*request);
    EnqueueArchiveCopy(request) => request.id, request.into();
    EnqueueCapturedTransfer(request) => request.id, request.into();
}

impl CopyInput {
    pub(super) fn initialize_captured_intent(&mut self) {
        if let Sources::Captured(sources) = &self.sources {
            self.intent = sources.intent();
        }
    }
    /// Reconcile the requested count with the existing native Copy de-duplication.
    /// Call only after validation so repeated paths cannot bypass the input bound.
    pub(super) fn normalize_copy_roots(&mut self) -> Option<u64> {
        if self.intent != TransferIntent::Copy {
            return None;
        }
        let files = match &self.sources {
            Sources::Paths(files) | Sources::SharedPaths(files) => files.as_ref(),
            _ => return None,
        };
        let mut seen = std::collections::HashSet::with_capacity(files.paths.len());
        if files.paths.iter().all(|path| seen.insert(path)) {
            return None;
        }
        seen.clear();
        let paths: Vec<_> = files
            .paths
            .iter()
            .filter(|path| seen.insert(*path))
            .cloned()
            .collect();
        let count = paths.len() as u64;
        self.sources = Sources::Paths(Arc::new(FileClipboard {
            paths,
            intent: files.intent,
        }));
        Some(count)
    }

    pub(super) fn native_children_are_audited(&mut self) -> bool {
        if let Some(checked) = self.audited_native_children {
            return checked;
        }
        let checked = match &self.sources {
            Sources::Captured(sources) => sources.prepares_audited_native_children(),
            _ => true,
        };
        self.audited_native_children = Some(checked);
        checked
    }
    pub(super) fn retirement_is_checked(&self) -> bool {
        // Native request fields are metadata. Captured/archive references have
        // explicit consuming owners; an unknown release trips the existing seal.
        true
    }
    pub(super) fn options(request: CopyOptionsRequest) -> Self {
        let mut input = match request.input {
            CopyOptionsInput::Native(request) => Self::metadata(request),
            CopyOptionsInput::Captured(request) => Self::from(request),
        };
        input.links = request.links;
        input
    }

    pub(super) fn metadata(request: MetadataTransferRequest) -> Self {
        let mut input = match request.input {
            MetadataTransferInput::Paths(paths) => Self::from(paths),
            MetadataTransferInput::Selected(selected) => Self::from(selected),
            MetadataTransferInput::Clipboard(clipboard) => Self::move_clipboard(clipboard),
        };
        input.intent = request.intent;
        input.metadata = request.metadata;
        input
    }
    pub(super) fn move_clipboard(request: ClipboardMoveRequest) -> Self {
        Self {
            id: request.id,
            destination: request.destination,
            intent: TransferIntent::Move,
            metadata: TransferMetadataPolicy::ContentOnly,
            verification: request.verification,
            start_policy: request.start_policy,
            links: CopyLinkPolicy::Reject,
            _completions: None,
            audited_native_children: None,
            sources: Sources::SharedPaths(request.clipboard),
        }
    }
    pub(super) fn move_paths(request: CopyRequest) -> Self {
        Self {
            intent: TransferIntent::Move,
            ..request.into()
        }
    }
    pub(super) fn move_selection(request: SelectedCopyRequest) -> Self {
        Self {
            intent: TransferIntent::Move,
            ..request.into()
        }
    }
    pub(super) fn requested_roots(&self) -> u64 {
        match &self.sources {
            Sources::Captured(sources) => sources.root_count() as u64,
            Sources::Paths(files) => files.paths.len() as u64,
            Sources::SharedPaths(files) => files.paths.len() as u64,
            Sources::Selection(files) => files
                .selection
                .len()
                .max(usize::from(files.fallback.is_some()))
                as u64,
            Sources::Archive(request) => request
                .selected
                .len()
                .max(usize::from(request.fallback.is_some()))
                as u64,
        }
    }

    pub(super) fn validate(&self) -> Result<(), FsError> {
        // Copy and Move keep links as links (Preserve) or copy what they point to
        // (FollowLinks; a Move then removes only the link). Required modified time
        // keeps links refused.
        if self.links.recreates_links() && matches!(self.sources, Sources::Archive(_)) {
            return Err(error(
                &self.destination,
                io::ErrorKind::Unsupported,
                "Archive sources do not support link preservation.",
            ));
        }
        let clipboard = match &self.sources {
            Sources::Paths(files) | Sources::SharedPaths(files) => Some(files.as_ref()),
            _ => None,
        };
        if let Some(files) = clipboard {
            files.validate()?;
            check_native_roots(&self.destination, files.paths.len())?;
            if files.intent == FileClipboardIntent::Move && self.intent == TransferIntent::Copy {
                return Err(error(
                    &self.destination,
                    io::ErrorKind::Unsupported,
                    "Move clipboard operations are not supported; sources have been retained. Use Copy instead.",
                ));
            }
        }
        Ok(())
    }

    /// Called only by blocking preparation, inside the existing queue reservation.
    pub(super) fn resolve(
        self,
        service: &dyn TransferService,
        cancel: &Cancellation,
    ) -> Result<Resolved, FsError> {
        self.validate()?;
        let links = source_link_policy(self.links, self.metadata);
        let clipboard = match self.sources {
            Sources::Captured(sources) => {
                if self.metadata != TransferMetadataPolicy::ContentOnly
                    && !sources.metadata_available()
                {
                    return Err(error(
                        &self.destination,
                        io::ErrorKind::Unsupported,
                        "Required modified time is unavailable for these sources.",
                    ));
                }
                let expected = sources.root_count();
                if sources.intent() != self.intent {
                    return Err(error(
                        &self.destination,
                        io::ErrorKind::InvalidData,
                        "Captured transfer intent changed.",
                    ));
                }
                check_required_metadata_roots(&self.destination, self.metadata, expected)?;
                let prepared = sources.prepare_with_links(service, links, cancel)?;
                let count = match &prepared {
                    PreparedCapturedTransfer::Copy(roots)
                        if self.intent == TransferIntent::Copy =>
                    {
                        roots.len()
                    }
                    PreparedCapturedTransfer::Move(roots)
                        if self.intent == TransferIntent::Move
                            && links == CopyLinkPolicy::Reject =>
                    {
                        roots.len()
                    }
                    PreparedCapturedTransfer::MoveWithDirectoryLinks(roots)
                        if self.intent == TransferIntent::Move
                            && links != CopyLinkPolicy::Reject =>
                    {
                        roots.len()
                    }
                    _ => {
                        return Err(error(
                            &self.destination,
                            io::ErrorKind::InvalidData,
                            "Prepared transfer authority differs from captured intent.",
                        ));
                    }
                };
                if count == 0 || count != expected {
                    return Err(error(
                        &self.destination,
                        io::ErrorKind::InvalidData,
                        "Prepared transfer root count differs from capture.",
                    ));
                }
                return Ok(Resolved::Captured {
                    destination: self.destination,
                    sources: prepared,
                });
            }
            Sources::Paths(files) | Sources::SharedPaths(files) => files,
            Sources::Selection(files) => {
                let paths = crate::file_targets::resolve_payload(
                    &files.selection,
                    &files.view,
                    files.fallback.as_deref(),
                    cancel,
                )?;
                check_native_roots(&self.destination, paths.len())?;
                Arc::new(FileClipboard {
                    paths,
                    intent: FileClipboardIntent::Copy,
                })
            }
            Sources::Archive(request) => {
                let sources = super::archive_source::resolve(&request, cancel)?;
                return Ok(Resolved::Archive {
                    destination: self.destination,
                    sources,
                });
            }
        };
        clipboard.validate()?;
        check_required_metadata_roots(&self.destination, self.metadata, clipboard.paths.len())?;
        // More items than a job prepares before it starts stream: the job takes them one
        // at a time. Required modified time prepares every root first (and takes few).
        if clipboard.paths.len() > MAX_PREPARED_NATIVE_ROOTS
            && self.metadata == TransferMetadataPolicy::ContentOnly
        {
            return Ok(Resolved::Streamed(StreamedRequest {
                destination: self.destination,
                verification: self.verification,
                paths: clipboard,
                moved: self.intent == TransferIntent::Move,
            }));
        }
        let request = CopyRequest {
            verification: self.verification,
            start_policy: self.start_policy,
            id: self.id,
            clipboard: Arc::unwrap_or_clone(clipboard),
            destination: self.destination,
        };
        Ok(if self.intent == TransferIntent::Move {
            Resolved::NativeMove(request)
        } else {
            Resolved::Native(request)
        })
    }
}
