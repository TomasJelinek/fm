//! One admission path for paste, drops and immutable panel selections.
use super::*;
use std::path::PathBuf;

enum Sources {
    Paths(FileClipboard),
    Selection(SelectedFiles),
    Archive(ArchiveCopyRequest),
}

impl From<ArchiveCopyRequest> for CopyInput {
    fn from(request: ArchiveCopyRequest) -> Self {
        Self {
            id: request.id,
            destination: request.destination.clone(),
            sources: Sources::Archive(request),
        }
    }
}

pub(super) enum Resolved {
    Native(CopyRequest),
    Archive {
        destination: PathBuf,
        sources: std::collections::VecDeque<Box<dyn TransferSource>>,
    },
}

pub(super) struct CopyInput {
    pub(super) id: OperationId,
    pub(super) destination: PathBuf,
    sources: Sources,
}

impl From<CopyRequest> for CopyInput {
    fn from(request: CopyRequest) -> Self {
        Self {
            id: request.id,
            destination: request.destination,
            sources: Sources::Paths(request.clipboard),
        }
    }
}

impl From<SelectedCopyRequest> for CopyInput {
    fn from(request: SelectedCopyRequest) -> Self {
        Self {
            id: request.id,
            destination: request.destination,
            sources: Sources::Selection(request.files),
        }
    }
}

impl CopyInput {
    pub(super) fn requested_roots(&self) -> u64 {
        match &self.sources {
            Sources::Paths(files) => files.paths.len() as u64,
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
        if let Sources::Paths(files) = &self.sources {
            files.validate()?;
            if files.intent == FileClipboardIntent::Move {
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
    pub(super) fn resolve(self, cancel: &Cancellation) -> Result<Resolved, FsError> {
        let clipboard = match self.sources {
            Sources::Paths(files) => files,
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
        Ok(Resolved::Native(CopyRequest {
            id: self.id,
            clipboard,
            destination: self.destination,
        }))
    }
}
