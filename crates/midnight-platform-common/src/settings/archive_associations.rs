//! Strict bounded map decoding: repeated keys must not silently overwrite choices.
use serde::{Deserialize, Serialize, de};
use std::{collections::BTreeMap, fmt};

#[derive(Serialize)]
#[serde(transparent)]
pub(super) struct StoredArchiveAssociations(pub BTreeMap<String, bool>);

impl<'de> Deserialize<'de> for StoredArchiveAssociations {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct AssociationsVisitor;
        impl<'de> de::Visitor<'de> for AssociationsVisitor {
            type Value = StoredArchiveAssociations;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a map of known archive extensions to booleans")
            }

            fn visit_map<M: de::MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut choices = BTreeMap::new();
                while let Some((extension, enabled)) = map.next_entry::<String, bool>()? {
                    if !midnight_core::ARCHIVE_ASSOCIATIONS
                        .iter()
                        .any(|association| association.extension == extension)
                    {
                        return Err(de::Error::custom("unknown archive association extension"));
                    }
                    if choices.insert(extension, enabled).is_some() {
                        return Err(de::Error::custom("duplicate archive association extension"));
                    }
                }
                Ok(StoredArchiveAssociations(choices))
            }
        }
        deserializer.deserialize_map(AssociationsVisitor)
    }
}
