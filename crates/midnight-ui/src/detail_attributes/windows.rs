use midnight_core::EntryMetadata;

pub(crate) fn fields(metadata: &EntryMetadata) -> Vec<(&'static str, String)> {
    let Some(attributes) = metadata.native_attributes else {
        return Vec::new();
    };
    let mut names = Vec::new();
    let mut known = 0;
    for (flag, name) in [
        (0x1, "ReadOnly"),
        (0x2, "Hidden"),
        (0x4, "System"),
        (0x10, "Directory"),
        (0x20, "Archive"),
        (0x40, "Device"),
        (0x80, "Normal"),
        (0x100, "Temporary"),
        (0x200, "Sparse"),
        (0x400, "Reparse"),
        (0x800, "Compressed"),
        (0x1000, "Offline"),
        (0x2000, "NotIndexed"),
        (0x4000, "Encrypted"),
        (0x8000, "Integrity"),
        (0x10000, "Virtual"),
        (0x20000, "NoScrub"),
        // Windows assigns these two meanings to the same context-dependent bit.
        (0x40000, "EA/RecallOnOpen"),
        (0x80000, "Pinned"),
        (0x100000, "Unpinned"),
        (0x400000, "RecallOnDataAccess"),
    ] {
        known |= flag;
        if attributes & flag != 0 {
            names.push(name);
        }
    }
    if attributes & !known != 0 {
        names.push("Unknown");
    }
    let text = if names.is_empty() {
        "None".into()
    } else {
        names.join(" ")
    };
    vec![("Attributes", text)]
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
        assert_eq!(
            fields(&metadata),
            [("Attributes", "ReadOnly Archive Compressed Unknown".into())]
        );
        assert!(fields(&EntryMetadata::default()).is_empty());
    }

    #[test]
    fn displays_extended_attributes_and_zero_without_numeric_masks() {
        for (value, expected) in [
            (0, "None"),
            (0x80, "Normal"),
            (0x0007_8000, "Integrity Virtual NoScrub EA/RecallOnOpen"),
            (0x0058_0000, "Pinned Unpinned RecallOnDataAccess"),
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
}
