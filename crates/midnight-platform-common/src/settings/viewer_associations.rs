//! Versioned host routing preferences; no installed plugin manifest is edited.
use super::invalid;
use midnight_core::ViewerAssociations;
use serde::{Deserialize, Serialize};
use std::{io, sync::Arc};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredAssociations {
    rules: Vec<Rule>,
    order: Vec<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Rule {
    plugin: String,
    extensions: Vec<String>,
}

pub(super) fn decode(
    version: u32,
    stored: Option<StoredAssociations>,
) -> io::Result<Arc<ViewerAssociations>> {
    match (version, stored) {
        (1..=26, None) => Ok(Arc::default()),
        (27.., Some(stored)) => {
            let mut result = ViewerAssociations {
                order: stored.order,
                ..Default::default()
            };
            for rule in stored.rules {
                if result
                    .overrides
                    .insert(rule.plugin, rule.extensions)
                    .is_some()
                {
                    return Err(invalid("duplicate viewer association ID"));
                }
            }
            result.validate().map_err(invalid)?;
            Ok(Arc::new(result))
        }
        _ => Err(invalid("missing or unexpected viewer associations")),
    }
}

impl From<&ViewerAssociations> for StoredAssociations {
    fn from(value: &ViewerAssociations) -> Self {
        Self {
            rules: value
                .overrides
                .iter()
                .map(|(id, extensions)| Rule {
                    plugin: id.clone(),
                    extensions: extensions.clone(),
                })
                .collect(),
            order: value.order.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{decode_preferences, encode_preferences};
    use midnight_core::{UserPreferences, ViewerAssociations};
    use serde_json::{Value, json};
    use std::sync::Arc;

    fn preferences() -> UserPreferences {
        UserPreferences {
            viewer_associations: Arc::new(ViewerAssociations {
                overrides: [
                    ("source".into(), vec!["rs".into(), "custom.rs".into()]),
                    ("missing".into(), vec![]),
                ]
                .into(),
                order: vec!["source".into(), "missing".into()],
            }),
            ..Default::default()
        }
    }

    #[test]
    fn viewer_associations_roundtrip_and_legacy_defaults() {
        let value = preferences();
        let encoded = encode_preferences(value.clone()).unwrap();
        let document: Value = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(document["version"], 27);
        assert_eq!(decode_preferences(&encoded).unwrap(), value);
        let legacy =
            decode_preferences(br#"{"version":1,"theme":"blue","density":"desktop"}"#).unwrap();
        assert_eq!(*legacy.viewer_associations, ViewerAssociations::default());
    }

    #[test]
    fn viewer_associations_reject_duplicate_missing_null_and_premature_rules() {
        let valid: Value =
            serde_json::from_slice(&encode_preferences(preferences()).unwrap()).unwrap();
        for invalid in [
            Value::Null,
            json!({}),
            json!({"rules":[{"plugin":"source","extensions":["RS"]}],"order":[]}),
            json!({"rules":[{"plugin":"source","extensions":[]},{"plugin":"source","extensions":[]}],"order":[]}),
            json!({"rules":[],"order":["a","a"]}),
            json!({"rules":[],"order":[],"extra":true}),
        ] {
            let mut document = valid.clone();
            document["viewer_associations"] = invalid;
            assert!(decode_preferences(&serde_json::to_vec(&document).unwrap()).is_err());
        }
        let mut missing = valid.clone();
        missing
            .as_object_mut()
            .unwrap()
            .remove("viewer_associations");
        assert!(decode_preferences(&serde_json::to_vec(&missing).unwrap()).is_err());
        let mut premature = valid;
        premature["version"] = 26.into();
        assert!(decode_preferences(&serde_json::to_vec(&premature).unwrap()).is_err());
    }
}
