//! Explicit native path selection. Picking never launches or modifies a file.
use crate::{Cancellation, ChecksumAlgorithm, FsError};
use std::{
    io,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathPickerKind {
    File,
    Directory,
    Program,
    Image,
    /// Open an existing SFV, MD5, SHA-1, SHA-256 or SHA-512 checksum file.
    ChecksumOpen,
    /// Choose an export destination; selection does not create or replace a file.
    ChecksumSave(ChecksumAlgorithm),
    /// Choose a text export destination. Picking never creates, truncates or grants
    /// overwrite permission; the export worker separately validates any conflict.
    TextSave,
}

impl PathPickerKind {
    /// Default suffix for save dialogs. None identifies an existing-item picker.
    pub const fn save_extension(self) -> Option<&'static str> {
        match self {
            Self::ChecksumSave(algorithm) => Some(algorithm.extension()),
            Self::TextSave => Some("txt"),
            _ => None,
        }
    }

    /// Initial editable name shared by native Save implementations.
    pub fn suggested_save_name(self) -> Option<String> {
        match self {
            Self::ChecksumSave(algorithm) => Some(format!("checksums.{}", algorithm.extension())),
            Self::TextSave => Some("file-list.txt".into()),
            _ => None,
        }
    }
}

/// UI-owned native window identity; platform implementations validate their variant.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PathPickerOwner {
    #[default]
    None,
    Windows(usize),
    X11(u64),
}

#[derive(Clone, Debug)]
pub struct PathPickerRequest {
    pub kind: PathPickerKind,
    pub title: String,
    pub initial_directory: Option<PathBuf>,
    pub owner: PathPickerOwner,
}

impl PathPickerRequest {
    pub fn validate(&self) -> Result<(), FsError> {
        if self.title.len() > 1024 || self.title.chars().any(char::is_control) {
            return Err(invalid(
                "The picker title must be a single line of at most 1024 bytes.",
            ));
        }
        if let Some(path) = &self.initial_directory {
            validate_picked_path(path)?;
        }
        Ok(())
    }
}

/// Reject malformed service results without normalizing or truncating native paths.
pub fn validate_picked_path(path: &Path) -> Result<(), FsError> {
    let bytes = path.as_os_str().as_encoded_bytes();
    if !path.is_absolute() || bytes.contains(&0) {
        return Err(invalid(
            "The selected path must be absolute and contain no NUL.",
        ));
    }
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(invalid(
            "The selected path exceeds the picker's 4 MiB resource budget.",
        ));
    }
    Ok(())
}

fn invalid(message: &str) -> FsError {
    FsError::from_io(
        "choose path",
        Path::new(""),
        io::Error::new(io::ErrorKind::InvalidInput, message),
    )
}

pub trait PathPickerService: Send + Sync + 'static {
    /// Runs on a dedicated bounded backend worker. None is ordinary cancellation.
    /// Cancellation requests native closure and retains ownership until it exits.
    /// A platform's modal Shell/extension UI may defer that request; implementations
    /// must not destroy windows or terminate threads to force it to complete.
    fn pick_path(
        &self,
        request: &PathPickerRequest,
        cancel: &Cancellation,
    ) -> Result<Option<PathBuf>, FsError>;
}
