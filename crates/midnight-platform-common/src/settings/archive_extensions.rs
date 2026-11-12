//! Strict custom suffix map: repeated/colliding keys and unknown decoders fail atomically.
use midnight_core::{
    ArchiveFormat, MAX_ARCHIVE_CUSTOM_EXTENSIONS, validate_archive_custom_extensions,
};
use serde::{Deserialize, Serialize, Serializer, de, ser::SerializeMap};
use std::{collections::BTreeMap, fmt};

pub(super) struct StoredArchiveExtensions(pub BTreeMap<String, ArchiveFormat>);

impl Serialize for StoredArchiveExtensions {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (extension, format) in &self.0 {
            map.serialize_entry(extension, format.id())?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for StoredArchiveExtensions {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ExtensionsVisitor;
        impl<'de> de::Visitor<'de> for ExtensionsVisitor {
            type Value = StoredArchiveExtensions;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter
                    .write_str("a map of canonical literal archive suffixes to known decoder IDs")
            }
            fn visit_map<M: de::MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut extensions = BTreeMap::new();
                while let Some((extension, id)) = map.next_entry::<String, String>()? {
                    if extensions.len() >= MAX_ARCHIVE_CUSTOM_EXTENSIONS {
                        return Err(de::Error::custom("Too many custom archive extensions"));
                    }
                    let format = ArchiveFormat::from_id(&id)
                        .ok_or_else(|| de::Error::custom("Unknown archive decoder ID"))?;
                    if extensions.insert(extension, format).is_some() {
                        return Err(de::Error::custom("Duplicate custom archive extension"));
                    }
                }
                validate_archive_custom_extensions(&extensions).map_err(de::Error::custom)?;
                Ok(StoredArchiveExtensions(extensions))
            }
        }
        deserializer.deserialize_map(ExtensionsVisitor)
    }
}
