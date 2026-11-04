//! Pure catalog normalization shared by native font adapters.
use fm_core::MAX_FONT_FAMILY_BYTES;
use std::collections::BTreeSet;

pub const MAX_FONT_FAMILIES: usize = 16_384;

pub fn normalize_families(names: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut unique = BTreeSet::new();
    for name in names {
        if !name.is_empty()
            && name.len() <= MAX_FONT_FAMILY_BYTES
            && !name.chars().any(char::is_control)
        {
            unique.insert(name);
        }
        if unique.len() == MAX_FONT_FAMILIES {
            break;
        }
    }
    let mut names: Vec<_> = unique.into_iter().collect();
    names.sort_by_cached_key(|name| (name.to_lowercase(), name.clone()));
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_is_unique_sorted_and_preserves_unicode_names() {
        let names = normalize_families(
            [
                "文档",
                "zeta",
                "Alpha",
                "文档",
                "é 🧪",
                "",
                "bad\0name",
                "line\nfeed",
            ]
            .map(str::to_owned),
        );
        assert_eq!(names, ["Alpha", "é 🧪", "zeta", "文档"]);
    }
    #[test]
    fn catalog_bounds_are_explicit_without_truncating_names() {
        let long = "字".repeat(171);
        assert!(normalize_families([long]).is_empty());
        assert_eq!(
            normalize_families((0..MAX_FONT_FAMILIES + 20).map(|i| format!("Font {i}"))).len(),
            MAX_FONT_FAMILIES
        );
    }
}
