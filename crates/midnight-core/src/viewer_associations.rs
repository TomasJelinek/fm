//! User routing preferences; manifests remain the source of default capabilities.
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ViewerAssociations {
    pub overrides: BTreeMap<String, Vec<String>>,
    pub order: Vec<String>,
}

impl ViewerAssociations {
    pub fn parse_extensions(text: &str) -> Result<Vec<String>, &'static str> {
        if text.len() > 32_768
            || text
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\t' | '\n' | '\r'))
        {
            return Err("Invalid or excessively long extension list.");
        }
        let mut result = Vec::new();
        for token in text
            .split(|c: char| c.is_whitespace() || matches!(c, ',' | ';'))
            .filter(|v| !v.is_empty())
        {
            let token = token
                .strip_prefix("*.")
                .or_else(|| token.strip_prefix('.'))
                .unwrap_or(token)
                .to_lowercase();
            validate_suffix(&token)?;
            if !result.contains(&token) {
                result.push(token);
            }
        }
        if result.len() > 512 {
            return Err("At most 512 extensions can be assigned to one viewer.");
        }
        Ok(result)
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        crate::validate_disabled_viewers(&self.order)?;
        crate::validate_disabled_viewers(&self.overrides.keys().cloned().collect::<Vec<_>>())?;
        let mut bytes = 0;
        for extensions in self.overrides.values() {
            if extensions.len() > 512 {
                return Err("At most 512 extensions can be assigned to one viewer.");
            }
            let mut seen = std::collections::HashSet::new();
            for suffix in extensions {
                validate_suffix(suffix)?;
                if suffix.to_lowercase() != *suffix || !seen.insert(suffix) {
                    return Err("Viewer extensions must be lowercase and unique.");
                }
                bytes += suffix.len();
            }
        }
        if bytes > 65_536 {
            return Err("Viewer associations exceed the settings budget.");
        }
        Ok(())
    }

    /// A positive rank is a literal filename suffix match. No match is `None`.
    pub fn suffix_match(&self, id: &str, defaults: &[String], name: &str) -> Option<usize> {
        let name = name.to_lowercase();
        self.overrides
            .get(id)
            .map_or(defaults, Vec::as_slice)
            .iter()
            .filter(|suffix| {
                !suffix.is_empty()
                    && name
                        .strip_suffix(&suffix.to_lowercase())
                        .is_some_and(|prefix| prefix.ends_with('.'))
            })
            .map(String::len)
            .max()
    }

    /// Higher ranks win. Unlisted providers retain their manifest precedence.
    pub fn priority(&self, id: &str) -> usize {
        self.order
            .iter()
            .position(|value| value == id)
            .map_or(0, |index| self.order.len() - index)
    }
}

fn validate_suffix(suffix: &str) -> Result<(), &'static str> {
    if suffix.is_empty()
        || suffix.len() > 64
        || suffix.split('.').any(str::is_empty)
        || suffix.chars().any(|c| {
            c.is_control()
                || c.is_whitespace()
                || matches!(
                    c,
                    '/' | '\\' | '*' | '?' | ':' | '<' | '>' | '|' | '"' | ',' | ';'
                )
        })
    {
        return Err("Use literal extensions such as rs or tar.gz, without paths or wildcards.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_normalizes_literal_suffixes_and_removes_duplicates() {
        assert_eq!(
            ViewerAssociations::parse_extensions(" .RS, *.rs; tar.GZ  日本語 ").unwrap(),
            ["rs", "tar.gz", "日本語"]
        );
        assert!(ViewerAssociations::parse_extensions("").unwrap().is_empty());
        assert_eq!(
            ViewerAssociations::parse_extensions("e\u{301}").unwrap(),
            ["e\u{301}"]
        );
        for invalid in ["*", "r?", "../rs", "a\\b", "a..b", "a:", "a\0b"] {
            assert!(
                ViewerAssociations::parse_extensions(invalid).is_err(),
                "{invalid:?}"
            );
        }
    }

    #[test]
    fn overrides_replace_defaults_and_match_only_filename_suffix_boundaries() {
        let mut value = ViewerAssociations::default();
        let defaults = vec!["md".into()];
        assert_eq!(
            value.suffix_match("viewer", &defaults, "README.MD"),
            Some(2)
        );
        assert_eq!(value.suffix_match("viewer", &defaults, "readmemd"), None);
        value
            .overrides
            .insert("viewer".into(), vec!["tar.gz".into(), "gz".into()]);
        assert_eq!(value.suffix_match("viewer", &defaults, "a.TAR.GZ"), Some(6));
        assert_eq!(value.suffix_match("viewer", &defaults, "README.md"), None);
        value.overrides.insert("viewer".into(), vec![]);
        assert_eq!(value.suffix_match("viewer", &defaults, "README.md"), None);
    }

    #[test]
    fn persisted_state_rejects_noncanonical_or_duplicate_data() {
        for suffixes in [vec!["RS"], vec!["rs", "rs"], vec!["../rs"]] {
            let value = ViewerAssociations {
                overrides: [(
                    "viewer".into(),
                    suffixes.into_iter().map(String::from).collect(),
                )]
                .into(),
                ..Default::default()
            };
            assert!(value.validate().is_err());
        }
        assert!(
            ViewerAssociations {
                order: vec!["a".into(), "a".into()],
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            ViewerAssociations {
                overrides: [("../a".into(), vec![])].into(),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
}
