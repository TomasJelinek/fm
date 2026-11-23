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
    /// Part of the work could not be counted (a folder that could not be read, a file
    /// without a length, a count stopped by a pause) or turned out larger than counted:
    /// the counts are what is known, an approximation that progress shows with "~" and
    /// raises as work beyond it completes (`cover`). Never set on an exact count.
    pub approximate: bool,
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
            EntryKind::File => match item.metadata.size {
                Some(size) => {
                    self.bytes = self.bytes.and_then(|total| total.checked_add(size));
                }
                // The file still counts; its unknown length makes the bytes approximate.
                None => self.approximate("A file's length is unknown."),
            },
            EntryKind::Directory => {
                if self.scope == DiscoveryScope::SelectedRoots {
                    self.bytes = None;
                }
            }
            // A link that the transfer keeps as a link writes no content bytes (a link it
            // follows is discovered as what it points to, a file or a folder), and an entry
            // of another kind is never copied. Neither makes the byte total unknown.
            EntryKind::Symlink | EntryKind::DirectoryLink | EntryKind::Other => {}
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
    /// Keep the counts so far as an approximation (`approximate`) instead of giving up
    /// the totals; the first reason is kept. Counting goes on where it can.
    pub fn approximate(&mut self, message: &str) {
        if !self.approximate {
            self.message = message.chars().take(512).collect();
        }
        self.approximate = true;
    }
    /// Work completed beyond the counted totals: raise each total to at least what is
    /// done, so that a share of them never exceeds the whole, and mark them approximate.
    pub fn cover(&mut self, files: u64, directories: u64, bytes: Option<u64>) {
        let mut grew = false;
        for (total, done) in [
            (&mut self.files, files),
            (&mut self.directories, directories),
        ] {
            if done > *total {
                *total = done;
                grew = true;
            }
        }
        if let (Some(total), Some(done)) = (self.bytes.as_mut(), bytes)
            && done > *total
        {
            *total = done;
            grew = true;
        }
        if grew {
            self.approximate("More was copied than was counted; the totals grew with it.");
        }
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
        // The known bytes stay, marked approximate; none is invented for the unknown file.
        assert_eq!(value.bytes, Some(5));
        assert!(value.approximate);
        assert_eq!(value.state, DiscoveryState::Complete);
        value.unavailable("source changed");
        value.finish();
        assert_eq!(value.state, DiscoveryState::Unavailable);
    }
    #[test]
    fn an_unknown_length_or_more_work_than_counted_makes_the_totals_approximate() {
        let mut value = OperationDiscovery::scanning(DiscoveryScope::Recursive);
        value.observe(&item(EntryKind::File, Some(5)));
        assert!(!value.approximate);
        value.observe(&item(EntryKind::File, None));
        value.finish();
        assert_eq!(value.state, DiscoveryState::Complete);
        assert!(value.approximate);
        assert_eq!((value.files, value.bytes), (2, Some(5)));
        value.cover(1, 0, Some(4));
        assert_eq!(
            (value.files, value.bytes),
            (2, Some(5)),
            "within the totals"
        );
        value.cover(3, 1, Some(9));
        assert_eq!(
            (value.files, value.directories, value.bytes),
            (3, 1, Some(9))
        );
        let mut exact = OperationDiscovery::scanning(DiscoveryScope::Recursive);
        exact.observe(&item(EntryKind::File, Some(5)));
        exact.finish();
        exact.cover(1, 0, Some(5));
        assert!(
            !exact.approximate,
            "work within an exact count keeps it exact"
        );
    }
    #[test]
    fn links_and_other_entries_add_no_bytes_and_keep_the_total_known() {
        let mut value = OperationDiscovery::scanning(DiscoveryScope::Recursive);
        value.observe(&item(EntryKind::Directory, None));
        value.observe(&item(EntryKind::File, Some(5)));
        value.observe(&item(EntryKind::Symlink, Some(40)));
        value.observe(&item(EntryKind::DirectoryLink, None));
        value.observe(&item(EntryKind::Other, None));
        value.finish();
        assert_eq!(value.state, DiscoveryState::Complete);
        assert_eq!((value.files, value.directories), (4, 1));
        assert_eq!(
            value.bytes,
            Some(5),
            "only regular files hold content bytes"
        );
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
