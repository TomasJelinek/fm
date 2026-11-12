use midnight_core::EntryMetadata;

/// Presentation of actual backend metadata, independent of names and selection.
pub(crate) fn dim_icon(metadata: &EntryMetadata) -> bool {
    const HIDDEN_OR_SYSTEM: u32 = 0x2 | 0x4;
    metadata
        .native_attributes
        .is_some_and(|attributes| attributes & HIDDEN_OR_SYSTEM != 0)
}

// Conventional attrib letters where available; other flags have distinct letters.
// The tooltip explains every present flag, including context-dependent Windows bits.
const FLAGS: &[(u32, char, &str)] = &[
    (0x1, 'R', "Read-only"),
    (0x2, 'H', "Hidden"),
    (0x4, 'S', "System"),
    (0x10, 'D', "Directory"),
    (0x20, 'A', "Archive"),
    (0x40, 'Y', "Device (reserved)"),
    (0x80, 'N', "Normal"),
    (0x100, 'T', "Temporary"),
    (0x200, 'F', "Sparse"),
    (0x400, 'L', "Reparse point"),
    (0x800, 'C', "Compressed"),
    (0x1000, 'O', "Offline"),
    (0x2000, 'I', "Not content indexed"),
    (0x4000, 'E', "Encrypted"),
    (0x8000, 'V', "Integrity stream"),
    (0x10000, 'Z', "Virtual (reserved)"),
    (0x20000, 'X', "No scrub data"),
    (0x40000, 'Q', "Extended attributes / recall on open"),
    (0x80000, 'P', "Pinned"),
    (0x100000, 'U', "Unpinned"),
    (0x400000, 'W', "Recall on data access"),
];

pub(crate) fn fields(metadata: &EntryMetadata) -> Vec<(&'static str, String)> {
    let Some(attributes) = metadata.native_attributes else {
        return Vec::new();
    };
    let mut text: String = FLAGS
        .iter()
        .filter(|(flag, _, _)| attributes & flag != 0)
        .map(|(_, letter, _)| letter)
        .collect();
    if has_unknown(attributes) {
        text.push('?');
    }
    if text.is_empty() {
        text.push('—');
    }
    vec![("Attributes", text)]
}

pub(crate) fn explanation(metadata: &EntryMetadata) -> String {
    let Some(attributes) = metadata.native_attributes else {
        return String::new();
    };
    let mut lines: Vec<_> = FLAGS
        .iter()
        .filter(|(flag, _, _)| attributes & flag != 0)
        .map(|(_, letter, name)| format!("{letter} — {name}"))
        .collect();
    if has_unknown(attributes) {
        lines.push("? — Other attributes reported by Windows".into());
    }
    if lines.is_empty() {
        lines.push("No attributes set".into());
    }
    lines.join("\n")
}

fn has_unknown(attributes: u32) -> bool {
    attributes & !FLAGS.iter().fold(0, |known, (flag, _, _)| known | flag) != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn displays_native_flags_without_losing_unknown_bits() {
        let metadata = EntryMetadata {
            native_attributes: Some(0x8000_0821),
            ..Default::default()
        };
        assert_eq!(fields(&metadata), [("Attributes", "RAC?".into())]);
        assert!(fields(&EntryMetadata::default()).is_empty());
    }

    #[test]
    fn displays_extended_attributes_and_zero_without_numeric_masks() {
        for (value, expected) in [
            (0, "—"),
            (0x80, "N"),
            (0x0007_8000, "VZXQ"),
            (0x0058_0000, "PUW"),
            (0x0020_0000, "?"),
        ] {
            assert_eq!(
                fields(&EntryMetadata {
                    native_attributes: Some(value),
                    ..Default::default()
                }),
                [("Attributes", expected.into())]
            );
        }
    }

    #[test]
    fn tooltip_explains_compact_letters_and_unknown_attributes() {
        let metadata = EntryMetadata {
            native_attributes: Some(0x8000_0821),
            ..Default::default()
        };
        assert_eq!(
            explanation(&metadata),
            "R — Read-only\nA — Archive\nC — Compressed\n? — Other attributes reported by Windows"
        );
        assert!(explanation(&EntryMetadata::default()).is_empty());
    }
}
