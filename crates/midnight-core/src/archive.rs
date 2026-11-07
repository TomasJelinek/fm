//! Read-only virtual locations never contain a guessed native member path.
use crate::{Cancellation, EntryKind, FsError};
use std::sync::Arc;

/// Persistent provider address. Raw member components belong to the provider;
/// they are never joined onto the native container path. Session IDs are absent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchiveAddress {
    pub provider: String,
    pub container: std::path::PathBuf,
    pub source_version: Vec<u8>,
    pub components: Vec<Vec<u8>>,
}
impl ArchiveAddress {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.provider.is_empty()
            || self.provider.len() > 128
            || !self
                .provider
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
        {
            return Err("Invalid archive provider identity.");
        }
        if !self.container.is_absolute()
            || self.container.as_os_str().as_encoded_bytes().contains(&0)
        {
            return Err("Invalid native archive container path.");
        }
        if self.source_version.is_empty() || self.source_version.len() > 256 {
            return Err("Invalid archive source revision.");
        }
        if self.components.len() > 256
            || self.components.iter().map(Vec::len).sum::<usize>() > 64 * 1024
            || self.components.iter().any(|part| {
                part.is_empty()
                    || part == b"."
                    || part == b".."
                    || part.iter().any(|b| b"/\\:\0".contains(b))
            })
        {
            return Err("Invalid archive member locator.");
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VirtualEntryId {
    pub mount: u64,
    pub entry: u64,
}

#[derive(Clone, Debug)]
pub struct ArchiveNode {
    pub id: VirtualEntryId,
    pub parent: u64,
    pub raw_name: Arc<[u8]>,
    pub display_name: String,
    pub kind: EntryKind,
    pub size: Option<u64>,
    pub readable: bool,
}

/// Host-owned session; only backend workers call these methods.
pub trait ArchiveMount: Send + Sync + std::fmt::Debug {
    fn id(&self) -> u64;
    fn list(&self, directory: u64, cancel: &Cancellation) -> Result<Vec<ArchiveNode>, FsError>;
    fn read(
        &self,
        entry: u64,
        offset: u64,
        length: u32,
        cancel: &Cancellation,
    ) -> Result<Vec<u8>, FsError>;
    fn validate(&self) -> Result<(), FsError>;
}

#[derive(Clone, Debug)]
pub struct ArchiveLocation {
    /// Absent for a saved tab awaiting backend restoration or an unavailable plugin.
    pub mount: Option<Arc<dyn ArchiveMount>>,
    pub address: ArchiveAddress,
    /// Root is 1; member and parent IDs are opaque within this mount.
    pub directory: u64,
    pub trail: Vec<(u64, String)>,
}
impl ArchiveLocation {
    /// Presentation only: provider members retain their opaque identities and
    /// must never be passed to native filesystem APIs using this string.
    pub fn display_text(&self) -> String {
        let native: std::path::PathBuf = self.address.container.components().collect();
        let mut text = native.to_string_lossy().into_owned();
        for (_, name) in self.trail.iter().skip(1) {
            if !text.ends_with(std::path::MAIN_SEPARATOR) {
                text.push(std::path::MAIN_SEPARATOR);
            }
            text.push_str(name);
        }
        text
    }

    pub fn unavailable(address: ArchiveAddress) -> Self {
        let mut trail = vec![(
            0,
            address
                .container
                .file_name()
                .unwrap_or(address.container.as_os_str())
                .to_string_lossy()
                .into_owned(),
        )];
        trail.extend(
            address
                .components
                .iter()
                .map(|name| (0, String::from_utf8_lossy(name).into_owned())),
        );
        Self {
            mount: None,
            address,
            directory: 0,
            trail,
        }
    }
    pub fn mount_id(&self) -> Option<u64> {
        self.mount.as_ref().map(|m| m.id())
    }
}

#[derive(Clone, Debug)]
pub enum ArchiveAction {
    Open { path: std::path::PathBuf },
    List(ArchiveLocation),
}

#[derive(Clone, Debug)]
pub struct ArchiveListing {
    pub location: ArchiveLocation,
    pub entries: Arc<[crate::Entry]>,
    pub index: Arc<crate::DirectoryIndex>,
}

#[derive(Clone, Debug)]
pub enum ArchiveSelectionAction {
    Toggle(crate::EntryId),
    Bulk(crate::SelectionOperation),
    Range {
        first: usize,
        last: usize,
        additive: bool,
    },
    Ranges {
        ranges: crate::SelectionRanges,
        mode: crate::SelectionMode,
    },
}

#[derive(Clone, Debug, Default)]
pub struct ArchiveSelectionResult {
    pub selected: Arc<std::collections::HashSet<crate::EntryId>>,
    pub totals: ArchiveSelectionTotals,
}

/// Cached totals for a provider selection. Bulk updates are prepared on workers;
/// a single cached row toggle updates these counters in constant time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ArchiveSelectionTotals {
    files: usize,
    folders: usize,
    known_bytes: u128,
    unknown_files: usize,
}
impl ArchiveSelectionTotals {
    pub fn files(self) -> usize {
        self.files
    }
    pub fn folders(self) -> usize {
        self.folders
    }
    pub fn file_bytes(self) -> Option<u64> {
        if self.unknown_files == 0 {
            u64::try_from(self.known_bytes).ok()
        } else {
            None
        }
    }
    pub fn change(&mut self, kind: EntryKind, size: Option<u64>, add: bool) {
        let adjust = |value: &mut usize| {
            *value = if add {
                value.saturating_add(1)
            } else {
                value.saturating_sub(1)
            };
        };
        if matches!(kind, EntryKind::Directory | EntryKind::DirectoryLink) {
            adjust(&mut self.folders);
        } else {
            adjust(&mut self.files);
            if kind == EntryKind::File
                && let Some(size) = size
            {
                self.known_bytes = if add {
                    self.known_bytes.saturating_add(u128::from(size))
                } else {
                    self.known_bytes.saturating_sub(u128::from(size))
                };
            } else {
                adjust(&mut self.unknown_files);
            }
        }
    }
}

#[cfg(test)]
mod selection_tests {
    use super::*;
    #[test]
    fn cached_totals_keep_unknown_and_overflow_distinct_and_recover_after_removal() {
        let mut totals = ArchiveSelectionTotals::default();
        totals.change(EntryKind::File, Some(u64::MAX), true);
        totals.change(EntryKind::File, Some(1), true);
        totals.change(EntryKind::Directory, None, true);
        assert_eq!(
            (totals.files(), totals.folders(), totals.file_bytes()),
            (2, 1, None)
        );
        totals.change(EntryKind::File, Some(1), false);
        assert_eq!(totals.file_bytes(), Some(u64::MAX));
        totals.change(EntryKind::Symlink, Some(1), true);
        assert_eq!(totals.file_bytes(), None);
        totals.change(EntryKind::Symlink, Some(1), false);
        assert_eq!(totals.file_bytes(), Some(u64::MAX));
    }
}
