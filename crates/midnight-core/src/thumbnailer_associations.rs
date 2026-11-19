//! Host routing choices keyed by the trusted thumbnail provider identifier.
use std::collections::{BTreeMap, BTreeSet};

/// The installed thumbnail catalog admits at most 128 entries.
pub const MAX_THUMBNAILER_OVERRIDES: usize = 128;
const MAX_EXTENSIONS: usize = 32;

/// A missing key uses the installed manifest. A present key replaces the
/// manifest list and an empty list disables that provider's associations.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThumbnailerAssociations {
    pub overrides: BTreeMap<String, Vec<String>>,
}

impl ThumbnailerAssociations {
    /// Read final extensions from a user-edited list. The input can include
    /// leading dots and conventional wildcard prefixes like .jpg or *.png.
    pub fn parse_extensions(text: &str) -> Result<Vec<String>, &'static str> {
        if text.len() > 4096
            || text
                .chars()
                .any(|character| character.is_control() && !matches!(character, '\t' | '\n' | '\r'))
        {
            return Err("Invalid or excessively long thumbnailer extension list.");
        }
        let mut extensions = Vec::new();
        for token in text
            .split(|character: char| character.is_whitespace() || matches!(character, ',' | ';'))
            .filter(|token| !token.is_empty())
        {
            let extension = token
                .strip_prefix("*.")
                .or_else(|| token.strip_prefix('.'))
                .unwrap_or(token);
            if !valid_extension(extension) {
                return Err(
                    "Use final extensions such as jpg or png without paths or compound suffixes.",
                );
            }
            let extension = extension.to_ascii_lowercase();
            if !extensions.contains(&extension) {
                if extensions.len() == MAX_EXTENSIONS {
                    return Err("At most 32 extensions can be assigned to one thumbnailer.");
                }
                extensions.push(extension);
            }
        }
        Ok(extensions)
    }

    /// Check the installed provider ID syntax and bound preference storage.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.overrides.len() > MAX_THUMBNAILER_OVERRIDES {
            return Err("At most 128 thumbnailer provider overrides can be saved.");
        }
        for (provider, extensions) in &self.overrides {
            if !valid_provider_id(provider) {
                return Err("Thumbnailer overrides must use a valid provider ID.");
            }
            if extensions.len() > MAX_EXTENSIONS {
                return Err("At most 32 extensions can be assigned to one thumbnailer.");
            }
            let mut seen = BTreeSet::new();
            if extensions.iter().any(|extension| {
                !valid_extension(extension) || !seen.insert(extension.to_ascii_lowercase())
            }) {
                return Err("Thumbnailer extensions must be unique final extensions.");
            }
        }
        Ok(())
    }

    /// Return the replacement list, including an explicit empty list, or defaults.
    pub fn effective_extensions<'a>(&'a self, id: &str, defaults: &'a [String]) -> &'a [String] {
        self.overrides.get(id).map_or(defaults, Vec::as_slice)
    }

    /// Remove the override and restore the installed defaults for this provider.
    pub fn reset(&mut self, id: &str) -> bool {
        self.overrides.remove(id).is_some()
    }
}

fn valid_provider_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && !matches!(id, "." | "..")
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
}

fn valid_extension(extension: &str) -> bool {
    !extension.is_empty()
        && extension.len() <= 16
        && extension
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnailer_overrides_replace_disable_and_reset_provider_defaults() {
        let mut value = ThumbnailerAssociations::default();
        let defaults = vec!["jpg".into()];
        assert_eq!(
            value.effective_extensions("image.lab", &defaults),
            &defaults
        );
        value
            .overrides
            .insert("image.lab".into(), vec!["bmp".into()]);
        assert_eq!(value.effective_extensions("image.lab", &defaults), &["bmp"]);
        value.overrides.insert("image.lab".into(), Vec::new());
        assert!(
            value
                .effective_extensions("image.lab", &defaults)
                .is_empty()
        );
        assert_eq!(value.effective_extensions("other", &defaults), &defaults);
        assert!(value.reset("image.lab"));
        assert_eq!(
            value.effective_extensions("image.lab", &defaults),
            &defaults
        );
    }

    #[test]
    fn thumbnailer_preferences_parse_case_insensitive_final_extensions() {
        assert_eq!(
            ThumbnailerAssociations::parse_extensions(".PNG; *.JpG, png heif").unwrap(),
            ["png", "jpg", "heif"],
        );
        assert!(
            ThumbnailerAssociations::parse_extensions("")
                .unwrap()
                .is_empty()
        );
        for invalid in ["tar.gz", "sub/png", "p?g", "日本語", ".", "x\0"] {
            assert!(ThumbnailerAssociations::parse_extensions(invalid).is_err());
        }
    }
}
