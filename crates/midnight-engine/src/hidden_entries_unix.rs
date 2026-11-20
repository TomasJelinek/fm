//! Linux and macOS: a name that starts with a dot is hidden, for files and folders
//! alike. Archive members follow the same rule through their listed names.
use midnight_core::Entry;
use std::os::unix::ffi::OsStrExt;

pub(super) fn is_hidden(entry: &Entry) -> bool {
    entry.name.as_bytes().first() == Some(&b'.')
}

#[cfg(test)]
mod tests {
    use super::super::{is_hidden, retain_visible};
    use midnight_core::{Entry, EntryId, EntryKind};
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt, path::PathBuf};

    fn entry(id: u64, name: &[u8], kind: EntryKind) -> Entry {
        Entry::new(
            EntryId(id),
            PathBuf::from("/folder").join(OsStr::from_bytes(name)),
            kind,
        )
    }

    #[test]
    fn dot_names_are_hidden_whatever_their_kind_attributes_or_encoding() {
        for (name, hidden) in [
            (&b".hidden"[..], true),
            (b".hidden-folder", true),
            (b"..strange", true),
            (b".\xff\xfe", true),
            (b"visible", false),
            (b"visible.txt", false),
            (b" .space", false),
            (b"\xff.raw", false),
            ("文档.txt".as_bytes(), false),
        ] {
            for kind in [EntryKind::File, EntryKind::Directory, EntryKind::Symlink] {
                let mut value = entry(1, name, kind);
                // Windows attributes never decide on Unix.
                value.metadata.native_attributes = Some(if hidden { 0 } else { 0x2 });
                assert_eq!(is_hidden(&value), hidden, "{name:?} {kind:?}");
            }
        }
    }

    #[test]
    fn retaining_visible_entries_keeps_their_order_and_counts_the_rest() {
        let names: [&[u8]; 5] = [b".a", b"b", b".c", b"d", b"e"];
        let mut entries: Vec<_> = names
            .iter()
            .enumerate()
            .map(|(index, name)| entry(index as u64, name, EntryKind::File))
            .collect();
        assert_eq!(retain_visible(&mut entries.clone(), false), 0);
        assert_eq!(retain_visible(&mut entries, true), 2);
        let kept: Vec<_> = entries.iter().map(|entry| entry.name.clone()).collect();
        assert_eq!(kept, ["b", "d", "e"]);
    }
}
