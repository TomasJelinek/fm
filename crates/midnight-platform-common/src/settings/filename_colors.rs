use super::{StoredPanelFilter, invalid};
use midnight_core::{FilenameColorRule, FilenameColorTarget, FilenameStyle, FilterMode, RgbColor};
use serde::{Deserialize, Serialize};
use std::io;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredFilenameColor {
    enabled: bool,
    mode: String,
    expression: String,
    case_sensitive: bool,
    target: String,
    #[serde(deserialize_with = "nullable_color")]
    foreground: Option<String>,
    #[serde(deserialize_with = "nullable_color")]
    background: Option<String>,
}

fn nullable_color<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(deserializer)
}

impl StoredFilenameColor {
    pub(super) fn into_rule(self) -> io::Result<FilenameColorRule> {
        let filter = StoredPanelFilter {
            enabled: true,
            mode: self.mode,
            expression: self.expression,
            case_sensitive: self.case_sensitive,
        }
        .into_filter()?;
        Ok(FilenameColorRule {
            enabled: self.enabled,
            mode: filter.mode,
            expression: filter.expression,
            case_sensitive: filter.case_sensitive,
            target: match self.target.as_str() {
                "files" => FilenameColorTarget::Files,
                "folders" => FilenameColorTarget::Folders,
                "all" => FilenameColorTarget::All,
                _ => return Err(invalid("unknown filename color target")),
            },
            style: FilenameStyle {
                foreground: self
                    .foreground
                    .as_deref()
                    .map(RgbColor::parse)
                    .transpose()
                    .map_err(invalid)?,
                background: self
                    .background
                    .as_deref()
                    .map(RgbColor::parse)
                    .transpose()
                    .map_err(invalid)?,
            },
        })
    }
    pub(super) fn from_rule(rule: FilenameColorRule) -> Self {
        Self {
            enabled: rule.enabled,
            mode: match rule.mode {
                FilterMode::Masks => "masks",
                FilterMode::Regex => "regex",
            }
            .into(),
            expression: rule.expression,
            case_sensitive: rule.case_sensitive,
            target: match rule.target {
                FilenameColorTarget::Files => "files",
                FilenameColorTarget::Folders => "folders",
                FilenameColorTarget::All => "all",
            }
            .into(),
            foreground: rule.style.foreground.map(RgbColor::hex),
            background: rule.style.background.map(RgbColor::hex),
        }
    }
}
