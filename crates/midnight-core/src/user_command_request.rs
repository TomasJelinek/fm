//! Immutable activation captures and bounded presentation contracts for Custom Commands.
use crate::{Entry, FsError, SelectedFiles, UserCommand, UserCommandId, UserCommandProcess};
use std::{fmt, path::PathBuf, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UserCommandRequestId(pub u64);
#[derive(Clone, Debug)]
pub struct UserCommandContext {
    /// None explicitly represents a virtual/provider location, never a guessed archive path.
    pub active: Option<PathBuf>,
    pub other: Option<PathBuf>,
    pub files: Option<SelectedFiles>,
    pub focus: Option<Entry>,
}
#[derive(Clone, Debug)]
pub struct UserCommandRequest {
    pub id: UserCommandRequestId,
    pub command: UserCommandId,
    pub configuration: Arc<Vec<UserCommand>>,
    pub context: UserCommandContext,
    /// Typed tree arguments resolve only during backend preparation.
    pub captured: Option<Arc<dyn crate::CapturedUserCommandSources>>,
}
impl UserCommandRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.id.0 == 0 {
            return Err("Command request IDs must be nonzero.");
        }
        crate::validate_user_commands(&self.configuration)?;
        if self.captured.is_some() && (self.context.files.is_some() || self.context.focus.is_some())
        {
            return Err(
                "Typed command sources cannot be combined with caller-provided file arguments.",
            );
        }

        if self.context.active.is_none()
            && (self.context.files.is_some() || self.context.focus.is_some())
        {
            return Err("Archive/provider entries require a future explicit native export policy.");
        }
        for path in [&self.context.active, &self.context.other]
            .into_iter()
            .flatten()
        {
            if !path.is_absolute() || path.as_os_str().as_encoded_bytes().contains(&0) {
                return Err("Captured locations require absolute native paths without NUL.");
            }
        }
        if self
            .context
            .files
            .as_ref()
            .is_some_and(|files| files.selection.len() > 4096)
        {
            return Err("Command activation exceeds the 4096 captured-target budget.");
        }
        let mut bytes = self
            .context
            .active
            .as_ref()
            .map_or(0, |path| path.as_os_str().as_encoded_bytes().len())
            .saturating_add(
                self.context
                    .other
                    .as_ref()
                    .map_or(0, |path| path.as_os_str().as_encoded_bytes().len()),
            );
        if let Some(focus) = &self.context.focus {
            bytes = bytes
                .saturating_add(focus.path.as_os_str().as_encoded_bytes().len())
                .saturating_add(focus.name.as_encoded_bytes().len())
                .saturating_add(focus.display_name.len());
        }
        if bytes > 8 * 1024 * 1024 {
            return Err("Command capture exceeds the 8 MiB input resource budget.");
        }
        Ok(())
    }
}
pub struct UserCommandRetention {
    _owner: Arc<dyn Send + Sync>,
}
impl UserCommandRetention {
    pub fn new(owner: impl Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self {
            _owner: Arc::new(owner),
        })
    }
}
impl fmt::Debug for UserCommandRetention {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output.write_str("UserCommandRetention")
    }
}
#[derive(Clone, Debug)]
pub struct UserCommandPreviewRow {
    pub process: UserCommandProcess,
    pub retention: Arc<UserCommandRetention>,
}
#[derive(Clone, Debug)]
pub struct UserCommandPreview {
    pub processes: Arc<[Arc<UserCommandPreviewRow>]>,
    pub retention: Arc<UserCommandRetention>,
}
#[derive(Clone, Debug)]
pub struct UserCommandSummary {
    pub planned: u64,
    pub started: u64,
    pub not_started: u64,
    pub cancelled: bool,
    pub error: Option<FsError>,
}
#[derive(Clone, Debug)]
pub enum UserCommandUpdate {
    Queued,
    Preparing,
    Preview(Arc<UserCommandPreview>),
    Started { index: u64, process_id: u32 },
    Rejected(FsError),
    Finished(UserCommandSummary),
}
#[derive(Clone, Debug)]
pub struct UserCommandEvent {
    pub request: UserCommandRequestId,
    pub command: UserCommandId,
    pub update: UserCommandUpdate,
}
