//! Windows: the hidden attribute decides, as in File Explorer; dot-names are
//! ordinary names. Native listings report attributes during enumeration; entries
//! without attributes (archive members, network locations) are never hidden.
use midnight_core::Entry;

const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;

pub(super) fn is_hidden(entry: &Entry) -> bool {
    entry
        .metadata
        .native_attributes
        .is_some_and(|attributes| attributes & FILE_ATTRIBUTE_HIDDEN != 0)
}

#[cfg(test)]
mod tests {
    use super::super::{is_hidden, retain_visible};
    use midnight_core::{Entry, EntryId, EntryKind};
    use std::path::PathBuf;

    fn entry(id: u64, name: &str, attributes: Option<u32>) -> Entry {
        let mut entry = Entry::new(
            EntryId(id),
            PathBuf::from("C:\\folder").join(name),
            EntryKind::File,
        );
        entry.metadata.native_attributes = attributes;
        entry
    }

    #[test]
    fn the_hidden_attribute_decides_and_dot_names_stay_visible() {
        for (name, attributes, hidden) in [
            (".dotted", None, false),
            (".dotted", Some(0x20), false),
            ("plain", Some(0x2), true),
            ("system", Some(0x4), false),
            ("both", Some(0x6), true),
            ("unknown", None, false),
        ] {
            assert_eq!(is_hidden(&entry(1, name, attributes)), hidden, "{name}");
        }
    }

    #[test]
    fn retaining_visible_entries_keeps_their_order_and_counts_the_rest() {
        let mut entries = vec![
            entry(1, "a", Some(0x2)),
            entry(2, "b", None),
            entry(3, ".c", Some(0x20)),
        ];
        assert_eq!(retain_visible(&mut entries, true), 1);
        let kept: Vec<_> = entries.iter().map(|entry| entry.name.clone()).collect();
        assert_eq!(kept, ["b", ".c"]);
    }
}
