//! Explicit provider-keyed overrides in preferences schema 34 and newer.
use super::invalid;
use midnight_core::{MAX_THUMBNAILER_OVERRIDES, ThumbnailerAssociations};
use serde::{Deserialize, Serialize, de};
use std::{collections::BTreeMap, fmt, io, sync::Arc};

#[derive(Serialize)]
#[serde(transparent)]
pub(super) struct StoredAssociations(pub BTreeMap<String, Vec<String>>);

impl<'de> Deserialize<'de> for StoredAssociations {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Overrides;
        impl<'de> de::Visitor<'de> for Overrides {
            type Value = StoredAssociations;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a map of thumbnailer provider IDs to final extension lists")
            }

            fn visit_map<M: de::MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut overrides = BTreeMap::new();
                while let Some((id, extensions)) = map.next_entry::<String, Vec<String>>()? {
                    if overrides.len() == MAX_THUMBNAILER_OVERRIDES {
                        return Err(de::Error::custom("too many thumbnailer overrides"));
                    }
                    if overrides.insert(id, extensions).is_some() {
                        return Err(de::Error::custom("duplicate thumbnailer provider ID"));
                    }
                }
                Ok(StoredAssociations(overrides))
            }
        }
        deserializer.deserialize_map(Overrides)
    }
}

pub(super) fn decode(
    version: u32,
    stored: Option<StoredAssociations>,
) -> io::Result<Arc<ThumbnailerAssociations>> {
    match (version, stored) {
        // Optional from schema 34 on: later schemas are written for other choices.
        (1..=super::CURRENT_PREFERENCES_VERSION, None) => Ok(Arc::default()),
        (34.., Some(stored)) => {
            let preferences = ThumbnailerAssociations {
                overrides: stored.0,
            };
            preferences.validate().map_err(invalid)?;
            Ok(Arc::new(preferences))
        }
        _ => Err(invalid("missing or unexpected thumbnailer associations")),
    }
}

#[cfg(test)]
mod tests {
    use super::super::{decode_preferences, encode_preferences};
    use midnight_core::{ThumbnailerAssociations, UserPreferences};
    use serde_json::{Value, json};
    use std::sync::Arc;

    fn preferences() -> UserPreferences {
        UserPreferences {
            thumbnailer_associations: Arc::new(ThumbnailerAssociations {
                overrides: [
                    ("empty.video".into(), Vec::new()),
                    ("image.lab".into(), vec!["JpG".into(), "bmp".into()]),
                ]
                .into(),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn schema_34_preserves_required_schema_33_fields_with_empty_comparators() {
        let preferences = preferences();
        let encoded = encode_preferences(preferences.clone()).unwrap();
        assert!(encoded.starts_with(b"{\n  \"version\": 34,"));
        let mut stored: Value = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(stored["version"], 34);
        assert_eq!(stored["file_comparators"], json!({"rules": []}));
        assert_eq!(stored["thumbnailer_associations"]["empty.video"], json!([]));
        assert_eq!(
            stored["thumbnailer_associations"]["image.lab"],
            json!(["JpG", "bmp"])
        );
        assert_eq!(decode_preferences(&encoded).unwrap(), preferences);

        stored["version"] = 33.into();
        stored
            .as_object_mut()
            .unwrap()
            .remove("thumbnailer_associations");
        let previous = decode_preferences(&serde_json::to_vec(&stored).unwrap()).unwrap();
        assert!(previous.thumbnailer_associations.overrides.is_empty());
        assert!(previous.file_comparators.rules.is_empty());
        let older =
            decode_preferences(br#"{"version":1,"theme":"blue","density":"desktop"}"#).unwrap();
        assert!(older.thumbnailer_associations.overrides.is_empty());
    }

    #[test]
    fn glyph_fitting_and_thumbnailer_overrides_round_trip_together() {
        let mut expected = preferences();
        expected.glyph_fitting = midnight_core::GlyphFitting::Subpixel;
        let encoded = encode_preferences(expected.clone()).unwrap();
        let stored: Value = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(stored["version"], 34);
        assert_eq!(stored["glyph_fitting"], "subpixel");
        assert_eq!(stored["thumbnailer_associations"]["empty.video"], json!([]));
        assert_eq!(decode_preferences(&encoded).unwrap(), expected);
    }

    #[test]
    fn thumbnailer_schema_rejects_malformed_duplicate_and_wrong_version_maps() {
        let encoded = encode_preferences(preferences()).unwrap();
        let duplicate = std::str::from_utf8(&encoded).unwrap().replace(
            "\"empty.video\": []",
            "\"empty.video\": [], \"empty.video\": []",
        );
        assert!(decode_preferences(duplicate.as_bytes()).is_err());

        let valid: Value = serde_json::from_slice(&encoded).unwrap();
        for invalid in [
            Value::Null,
            json!([]),
            json!({"../worker": []}),
            json!({"image.lab": ["png", "PNg"]}),
            json!({"image.lab": ["tar.gz"]}),
        ] {
            let mut mutated = valid.clone();
            mutated["thumbnailer_associations"] = invalid;
            assert!(decode_preferences(&serde_json::to_vec(&mutated).unwrap()).is_err());
        }

        let mut missing = valid.clone();
        missing
            .as_object_mut()
            .unwrap()
            .remove("thumbnailer_associations");
        assert!(decode_preferences(&serde_json::to_vec(&missing).unwrap()).is_err());

        let mut premature = valid;
        premature["version"] = 33.into();
        assert!(decode_preferences(&serde_json::to_vec(&premature).unwrap()).is_err());
    }

    #[test]
    fn reset_of_the_last_thumbnailer_provider_restores_legacy_minimum_schema() {
        let mut preferences = preferences();
        let overrides = Arc::make_mut(&mut preferences.thumbnailer_associations);
        assert!(overrides.reset("empty.video"));
        assert!(overrides.reset("image.lab"));
        let encoded = encode_preferences(preferences.clone()).unwrap();
        let stored: Value = serde_json::from_slice(&encoded).unwrap();
        assert_ne!(stored["version"], 34);
        assert!(stored.get("thumbnailer_associations").is_none());
        assert_eq!(decode_preferences(&encoded).unwrap(), preferences);
    }
}
