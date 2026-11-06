//! Backend-only native context menus. UI supplies captured window geometry and intent.

use crate::FsError;
use std::{io, path::PathBuf};

/// Physical desktop pixels, including negative coordinates on secondary monitors.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShellMenuPosition {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShellMenuModifiers {
    pub shift: bool,
    pub control: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellMenuTarget {
    Items,
    Drive,
}

#[derive(Clone, Debug)]
pub struct ShellMenuRequest {
    /// Immutable native paths expanded and validated on the backend.
    pub paths: Vec<PathBuf>,
    /// Borrowed native window identity. Zero is permitted for an unowned menu.
    pub owner: usize,
    pub position: ShellMenuPosition,
    pub modifiers: ShellMenuModifiers,
    pub target: ShellMenuTarget,
}

impl ShellMenuRequest {
    /// Resource limits reject the entire request; paths are never shortened or normalized.
    pub fn validate(&self) -> Result<(), FsError> {
        let invalid = |message| {
            FsError::from_io(
                "show system menu",
                self.paths
                    .first()
                    .map_or(std::path::Path::new(""), PathBuf::as_path),
                io::Error::new(io::ErrorKind::InvalidInput, message),
            )
        };
        if self.paths.is_empty() || self.paths.len() > 4096 {
            return Err(invalid("A system menu requires between 1 and 4096 items."));
        }
        if self.target == ShellMenuTarget::Drive && self.paths.len() != 1 {
            return Err(invalid("A drive menu requires exactly one drive root."));
        }
        let mut bytes = 0usize;
        for path in &self.paths {
            let native = path.as_os_str().as_encoded_bytes();
            if !path.is_absolute() || native.contains(&0) {
                return Err(invalid(
                    "System menu paths must be absolute and contain no NUL.",
                ));
            }
            bytes = bytes.saturating_add(native.len());
            if bytes > 4 * 1024 * 1024 {
                return Err(invalid(
                    "System menu paths exceed their 4 MiB resource budget.",
                ));
            }
            if self.target == ShellMenuTarget::Items && path.parent() != self.paths[0].parent() {
                return Err(invalid(
                    "A system menu selection must have a common parent folder.",
                ));
            }
        }
        Ok(())
    }
}

/// These commands must return to the host before any native mutation/copy occurs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellMenuVerb {
    Copy,
    Cut,
    Paste,
    Delete,
    Rename,
}

impl ShellMenuVerb {
    pub fn from_canonical(verb: &str) -> Option<Self> {
        [
            ("copy", Self::Copy),
            ("cut", Self::Cut),
            ("paste", Self::Paste),
            ("delete", Self::Delete),
            ("rename", Self::Rename),
        ]
        .into_iter()
        .find_map(|(name, value)| verb.eq_ignore_ascii_case(name).then_some(value))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellMenuOutcome {
    Dismissed,
    /// A Shell extension command was invoked; this does not assert its async completion.
    Invoked,
    Managed(ShellMenuVerb),
    /// The platform has no supported native context-menu embedding contract.
    Unavailable,
}

pub trait ShellMenuService: Send + Sync + 'static {
    /// Blocking native UI belongs to a bounded backend worker, never the application UI.
    /// Implementations keep thread-affine native objects on their own event-loop thread.
    fn show_shell_menu(&self, request: &ShellMenuRequest) -> Result<ShellMenuOutcome, FsError>;
}
