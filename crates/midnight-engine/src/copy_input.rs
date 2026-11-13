//! One admission path for paste, drops and immutable panel selections.
use super::*;
use std::path::PathBuf;

#[derive(Clone)]
enum Sources {
    Paths(FileClipboard),
    SharedPaths(Arc<FileClipboard>),
    Selection(SelectedFiles),
    Archive(Box<ArchiveCopyRequest>),
    Captured(Arc<dyn CapturedTransferSources>),
}

impl From<CapturedTransferRequest> for CopyInput {
    fn from(request: CapturedTransferRequest) -> Self {
        Self {
            id: request.id,
            destination: request.destination,
            intent: request.sources.intent(),
            metadata: request.metadata,
            verification: request.verification,
            links: CopyLinkPolicy::Reject,
            _retention: None,
            sources: Sources::Captured(request.sources),
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
            links: CopyLinkPolicy::Reject,
            _retention: None,
            sources: Sources::Archive(Box::new(request)),
        }
    }
}

pub(super) enum Resolved {
    Native(CopyRequest),
    NativeMove(CopyRequest),
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
    // Frozen at admission with the source capture and reused on every Retry.
    pub(super) links: CopyLinkPolicy,
    sources: Sources,
    // Preparation retains failed/retried admission and hands successful ownership
    // to Job. WorkerOwned keeps cancellation destruction off UI.
    pub(super) _retention: Option<Arc<crate::drag_drop::Retention>>,
}

impl From<CopyRequest> for CopyInput {
    fn from(request: CopyRequest) -> Self {
        Self {
            id: request.id,
            destination: request.destination,
            intent: TransferIntent::Copy,
            metadata: TransferMetadataPolicy::ContentOnly,
            verification: request.verification,
            links: CopyLinkPolicy::Reject,
            _retention: None,
            sources: Sources::Paths(request.clipboard),
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
            links: CopyLinkPolicy::Reject,
            _retention: None,
            sources: Sources::Selection(request.files),
        }
    }
}

impl CopyInput {
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
            links: CopyLinkPolicy::Reject,
            _retention: None,
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
        if self.links == CopyLinkPolicy::Preserve {
            if self.intent != TransferIntent::Copy {
                return Err(error(
                    &self.destination,
                    io::ErrorKind::Unsupported,
                    "Link preservation is available only for Copy; sources have been retained.",
                ));
            }
            if self.metadata != TransferMetadataPolicy::ContentOnly {
                return Err(error(
                    &self.destination,
                    io::ErrorKind::Unsupported,
                    "Link preservation cannot require modified time.",
                ));
            }
            if matches!(self.sources, Sources::Archive(_)) {
                return Err(error(
                    &self.destination,
                    io::ErrorKind::Unsupported,
                    "Archive sources do not support link preservation.",
                ));
            }
        }
        let clipboard = match &self.sources {
            Sources::Paths(files) => Some(files),
            Sources::SharedPaths(files) => Some(files.as_ref()),
            _ => None,
        };
        if let Some(files) = clipboard {
            files.validate()?;
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
                let prepared = sources.prepare_with_links(service, self.links, cancel)?;
                let count = match &prepared {
                    PreparedCapturedTransfer::Copy(roots)
                        if self.intent == TransferIntent::Copy =>
                    {
                        roots.len()
                    }
                    PreparedCapturedTransfer::Move(roots)
                        if self.intent == TransferIntent::Move =>
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
            Sources::Paths(files) => files,
            Sources::SharedPaths(files) => Arc::unwrap_or_clone(files),
            Sources::Selection(files) => FileClipboard {
                paths: crate::file_targets::resolve(
                    &files.selection,
                    &files.view,
                    files.fallback.as_deref(),
                    cancel,
                )?,
                intent: FileClipboardIntent::Copy,
            },
            Sources::Archive(request) => {
                let sources = super::archive_source::resolve(&request, cancel)?;
                return Ok(Resolved::Archive {
                    destination: self.destination,
                    sources,
                });
            }
        };
        clipboard.validate()?;
        let request = CopyRequest {
            verification: self.verification,
            id: self.id,
            clipboard,
            destination: self.destination,
        };
        Ok(if self.intent == TransferIntent::Move {
            Resolved::NativeMove(request)
        } else {
            Resolved::Native(request)
        })
    }
}
