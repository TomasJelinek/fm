//! System and per-user font directories are enumerated only on a backend worker.
pub(crate) fn families() -> Vec<String> {
    let mut database = fontdb::Database::new();
    database.load_system_fonts();
    midnight_platform_common::fonts::normalize_families(
        database
            .faces()
            .flat_map(|face| face.families.iter().map(|(name, _)| name.clone())),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn native_catalog_has_valid_unique_family_names() {
        let families = super::families();
        assert!(families.len() <= midnight_platform_common::fonts::MAX_FONT_FAMILIES);
        assert_eq!(
            families,
            midnight_platform_common::fonts::normalize_families(families.clone())
        );
        for family in families {
            midnight_core::UserPreferences {
                font_family: family,
                ..Default::default()
            }
            .validate()
            .unwrap();
        }
    }
}
