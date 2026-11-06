//! Typed, bounded metadata edits. Native identities and handles remain on workers.
use crate::{FsError, OperationId};
use std::{ffi::OsString, path::PathBuf, time::SystemTime};

pub const EDIT_READ_ONLY: u32 = 1;
pub const EDIT_HIDDEN: u32 = 2;
pub const EDIT_SYSTEM: u32 = 4;
pub const EDIT_ARCHIVE: u32 = 32;
pub const MAX_METADATA_TARGETS: usize = 256;
pub const MAX_METADATA_DEPTH: usize = 64;
pub const MAX_METADATA_NAMES_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TimeEdit {
    #[default]
    Keep,
    Set(SystemTime),
    /// Resolved once by the backend before publishing the confirming preview.
    Now,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CaseMode {
    #[default]
    Lower,
    Upper,
    Mixed,
    MixedNameLowerExtension,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CaseScope {
    #[default]
    Whole,
    Basename,
    Extension,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CaseChange {
    pub mode: CaseMode,
    pub scope: CaseScope,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MetadataPatch {
    pub windows_mask: u32,
    pub windows_value: u32,
    pub unix_mask: u32,
    pub unix_value: u32,
    pub compressed: Option<bool>,
    pub created: TimeEdit,
    pub modified: TimeEdit,
    pub accessed: TimeEdit,
    pub case: Option<CaseChange>,
    pub recursive: bool,
}

impl MetadataPatch {
    pub fn is_empty(&self) -> bool {
        self.windows_mask == 0
            && self.unix_mask == 0
            && self.compressed.is_none()
            && self.created == TimeEdit::Keep
            && self.modified == TimeEdit::Keep
            && self.accessed == TimeEdit::Keep
            && self.case.is_none()
    }

    pub fn resolve_now(&mut self, now: SystemTime) {
        for time in [&mut self.created, &mut self.modified, &mut self.accessed] {
            if *time == TimeEdit::Now {
                *time = TimeEdit::Set(now);
            }
        }
    }

    pub fn has_metadata_changes(&self) -> bool {
        let mut without_case = self.clone();
        without_case.case = None;
        !without_case.is_empty()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MetadataCapabilities {
    pub windows_attributes: u32,
    pub unix_permissions: u32,
    pub compressed: bool,
    pub created: bool,
    pub modified: bool,
    pub accessed: bool,
    pub rename: bool,
}

#[derive(Clone, Debug, Default)]
pub struct MetadataSnapshot {
    pub capabilities: MetadataCapabilities,
    pub windows_attributes: Option<u32>,
    pub unix_mode: Option<u32>,
    pub compressed: Option<bool>,
    pub encrypted: Option<bool>,
    pub created: Option<SystemTime>,
    pub modified: Option<SystemTime>,
    pub accessed: Option<SystemTime>,
    pub hard_links: u64,
}

#[derive(Clone, Debug)]
pub struct MetadataPreviewRow {
    pub path: PathBuf,
    pub proposed_name: Option<OsString>,
    pub current: MetadataSnapshot,
    pub issue: Option<FsError>,
}

#[derive(Clone, Debug)]
pub struct MetadataPreview {
    pub id: OperationId,
    pub revision: u64,
    pub patch: MetadataPatch,
    /// Capabilities common to all selected roots. Descendants are checked when discovered.
    pub capabilities: MetadataCapabilities,
    pub total: u64,
    pub conflicts: u64,
    pub unsupported: u64,
    pub hard_linked: u64,
    /// At most 32 rows; native names must never be reconstructed from display strings.
    pub sample: Vec<MetadataPreviewRow>,
    /// At most 16 preparation issues.
    pub errors: Vec<FsError>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MetadataConflictPolicy {
    #[default]
    Skip,
    SkipAll,
    Cancel,
}

#[derive(Clone, Debug)]
pub struct MetadataConflict {
    pub id: OperationId,
    pub token: u64,
    pub source: PathBuf,
    pub destination: PathBuf,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MetadataChanges {
    pub renamed: bool,
    pub attributes: bool,
    pub permissions: bool,
    pub compressed: bool,
    pub created: bool,
    pub modified: bool,
    pub accessed: bool,
}

impl MetadataChanges {
    pub fn any(self) -> bool {
        self.renamed
            || self.attributes
            || self.permissions
            || self.compressed
            || self.created
            || self.modified
            || self.accessed
    }
}

#[derive(Clone, Debug)]
pub struct MetadataOutcome {
    pub original_path: PathBuf,
    pub current_path: PathBuf,
    pub changes: MetadataChanges,
    pub error: Option<FsError>,
}

impl MetadataOutcome {
    pub fn unchanged(path: PathBuf) -> Self {
        Self {
            original_path: path.clone(),
            current_path: path,
            changes: MetadataChanges::default(),
            error: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct MetadataResult {
    pub id: OperationId,
    pub processed: u64,
    pub changed: u64,
    pub skipped: u64,
    pub failed: u64,
    /// At most 32 item outcomes, including partial changes. Totals are never truncated.
    pub sample: Vec<MetadataOutcome>,
}
