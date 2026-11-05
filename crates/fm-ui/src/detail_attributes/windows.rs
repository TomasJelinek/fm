use fm_core::EntryMetadata;

pub(crate) fn fields(metadata: &EntryMetadata) -> Vec<(&'static str, String)> {
    let Some(attributes) = metadata.native_attributes else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for (flag, name) in [
        (0x1, "R"),
        (0x2, "H"),
        (0x4, "S"),
        (0x10, "D"),
        (0x20, "A"),
        (0x100, "T"),
        (0x200, "Sparse"),
        (0x400, "Link"),
        (0x800, "C"),
        (0x1000, "Offline"),
        (0x2000, "NotIndexed"),
        (0x4000, "E"),
    ] {
        if attributes & flag != 0 {
            names.push(name);
        }
    }
    let text = if names.is_empty() {
        format!("0x{attributes:08X}")
    } else {
        format!("{} (0x{attributes:08X})", names.join(" "))
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
            [("Attributes", "R A C (0x80000821)".into())]
        );
        assert!(fields(&EntryMetadata::default()).is_empty());
    }
}
