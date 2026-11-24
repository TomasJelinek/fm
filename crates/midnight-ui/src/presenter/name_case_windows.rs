//! Windows folders name their entries without regard to letter case: two names are the
//! same when they are equal after an ordinal comparison of their UTF-16 units, each
//! through its simple uppercase mapping, as the NTFS upcase table maps them.
use midnight_core::Entry;
use std::{ffi::OsStr, os::windows::ffi::OsStrExt};

/// The one entry of `entries` whose name is `typed` in other letter case. Two or more
/// such entries (a folder that keeps letter case apart) name none of them.
pub(super) fn case_variant<'a>(entries: &'a [Entry], typed: &OsStr) -> Option<&'a Entry> {
    let mut named = entries.iter().filter(|entry| same_name(&entry.name, typed));
    let entry = named.next()?;
    named.next().is_none().then_some(entry)
}

/// Whether `entries` has a file (not a folder) whose name is `typed` in any letter
/// case. One pass, comparing UTF-16 units in place: no name is copied or case-mapped.
pub(super) fn lists_file_in_other_case(entries: &[Entry], typed: &OsStr) -> bool {
    entries.iter().any(|entry| {
        !matches!(
            entry.kind,
            midnight_core::EntryKind::Directory | midnight_core::EntryKind::DirectoryLink
        ) && same_name(&entry.name, typed)
    })
}

fn same_name(listed: &OsStr, typed: &OsStr) -> bool {
    let (mut listed, mut typed) = (listed.encode_wide(), typed.encode_wide());
    loop {
        match (listed.next(), typed.next()) {
            (None, None) => return true,
            (Some(a), Some(b)) if a == b || upcase(a) == upcase(b) => {}
            _ => return false,
        }
    }
}

/// The unit's uppercase when that is a single character of the basic plane; any other
/// unit, a surrogate or one whose uppercase is longer (`ß`), is its own uppercase.
fn upcase(unit: u16) -> u16 {
    let Some(character) = char::from_u32(u32::from(unit)) else {
        return unit;
    };
    let mut upper = character.to_uppercase();
    match (upper.next(), upper.next()) {
        (Some(upper), None) => u16::try_from(u32::from(upper)).unwrap_or(unit),
        _ => unit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use midnight_core::{EntryId, EntryKind};
    use std::path::Path;

    fn entries(names: &[&str]) -> Vec<Entry> {
        names
            .iter()
            .zip(1..)
            .map(|(name, id)| {
                Entry::new(
                    EntryId(id),
                    Path::new("C:\\složka").join(name),
                    EntryKind::File,
                )
            })
            .collect()
    }

    #[test]
    fn names_match_through_simple_uppercase_of_each_utf16_unit() {
        for (listed, typed) in [
            ("readme.txt", "README.TXT"),
            ("čti mě.txt", "ČTI MĚ.TXT"),
            ("σοφία ς.txt", "ΣΟΦΊΑ Σ.TXT"),
            ("ǆungla", "ǅUNGLA"),
            ("文档 🧪.txt", "文档 🧪.TXT"),
        ] {
            assert!(same_name(OsStr::new(listed), OsStr::new(typed)), "{typed}");
        }
        for (listed, typed) in [
            // Full case mappings and folding are not Windows name comparison.
            ("straße", "STRASSE"),
            ("ﬁle", "FILE"),
            // Letters outside the basic plane, and combining marks, stay as they are.
            ("\u{10428}", "\u{10400}"),
            ("e\u{301}.txt", "é.txt"),
            ("a.txt", "a.txt "),
        ] {
            assert!(!same_name(OsStr::new(listed), OsStr::new(typed)), "{typed}");
        }
    }

    #[test]
    fn only_a_single_entry_in_other_letter_case_is_named() {
        let listed = entries(&["Jiný.txt", "čti mě.txt", "ČTI.txt"]);
        let entry = case_variant(&listed, OsStr::new("ČTI MĚ.TXT")).unwrap();
        assert_eq!(entry.id, EntryId(2));
        assert!(case_variant(&listed, OsStr::new("chybí.txt")).is_none());
        // A folder that keeps letter case apart can list both: neither is chosen.
        let listed = entries(&["čti mě.txt", "Čti Mě.txt"]);
        assert!(case_variant(&listed, OsStr::new("ČTI MĚ.TXT")).is_none());
    }
}
