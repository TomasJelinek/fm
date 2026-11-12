//! Bounded metadata discovery snapshots, separate from authoritative mutation capabilities.
use crate::{EntryKind, MutationItem};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DiscoveryState {
    #[default]
    NotStarted,
    Scanning,
    Complete,
    /// Counts may be partial, unsupported or superseded; they are not a denominator.
    Unavailable,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DiscoveryScope {
    #[default]
    Recursive,
    /// A native rename/trash may process a directory as one selected root without scanning it.
    SelectedRoots,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OperationDiscovery {
    pub state: DiscoveryState,
    pub scope: DiscoveryScope,
    /// Non-directory entries, including links that are never followed recursively.
    pub files: u64,
    pub directories: u64,
    /// Logical regular-file lengths, never actual I/O or an estimate for unknown contents.
    pub bytes: Option<u64>,
    pub message: String,
}
impl OperationDiscovery {
    pub fn scanning(scope: DiscoveryScope) -> Self {
        Self {
            state: DiscoveryState::Scanning,
            scope,
            bytes: Some(0),
            ..Self::default()
        }
    }
    pub fn entries(&self) -> Option<u64> {
        self.files.checked_add(self.directories)
    }
    pub fn observe(&mut self, item: &MutationItem) {
        let count = if item.kind == EntryKind::Directory {
            &mut self.directories
        } else {
            &mut self.files
        };
        if let Some(next) = count.checked_add(1) {
            *count = next;
        } else {
            self.unavailable("Discovered entry count overflowed.");
            return;
        }
        match item.kind {
            EntryKind::File => {
                self.bytes = self
                    .bytes
                    .zip(item.metadata.size)
                    .and_then(|(total, size)| total.checked_add(size));
            }
            EntryKind::Directory => {
                if self.scope == DiscoveryScope::SelectedRoots {
                    self.bytes = None;
                }
            }
            _ => self.bytes = None,
        }
    }
    pub fn finish(&mut self) {
        if self.state == DiscoveryState::Scanning {
            self.state = DiscoveryState::Complete;
        }
    }
    pub fn unavailable(&mut self, message: &str) {
        self.state = DiscoveryState::Unavailable;
        self.message = message.chars().take(512).collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn item(kind: EntryKind, size: Option<u64>) -> MutationItem {
        MutationItem {
            path: "item".into(),
            kind,
            metadata: crate::EntryMetadata {
                size,
                ..Default::default()
            },
        }
    }
    #[test]
    fn partial_and_unknown_counts_never_become_fabricated_byte_totals() {
        let mut value = OperationDiscovery::scanning(DiscoveryScope::Recursive);
        value.observe(&item(EntryKind::Directory, None));
        value.observe(&item(EntryKind::File, Some(5)));
        assert_eq!(value.state, DiscoveryState::Scanning);
        value.observe(&item(EntryKind::File, None));
        value.finish();
        assert_eq!(value.entries(), Some(3));
        assert_eq!(value.bytes, None);
        assert_eq!(value.state, DiscoveryState::Complete);
        value.unavailable("source changed");
        value.finish();
        assert_eq!(value.state, DiscoveryState::Unavailable);
    }
    #[test]
    fn native_root_scope_never_invents_directory_contents() {
        let mut value = OperationDiscovery::scanning(DiscoveryScope::SelectedRoots);
        value.observe(&item(EntryKind::Directory, Some(4096)));
        value.finish();
        assert_eq!(value.entries(), Some(1));
        assert_eq!(value.bytes, None);
    }
}
