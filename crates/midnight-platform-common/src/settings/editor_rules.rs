//! Version 30 editor associations, with the previous editor retained as fallback.
use super::{NativePath, invalid};
use midnight_core::{EditorRule, FilterMode};
use serde::{Deserialize, Serialize};
use std::io;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredEditorRule {
    enabled: bool,
    mode: String,
    expression: String,
    case_sensitive: bool,
    executable: Option<NativePath>,
    arguments: Vec<String>,
    working_directory: Option<NativePath>,
}

pub(super) fn decode(
    version: u32,
    rules: Option<Vec<StoredEditorRule>>,
) -> io::Result<Vec<EditorRule>> {
    match (version, rules) {
        (1..=29, None) => Ok(Vec::new()),
        (30.., Some(rules)) => rules
            .into_iter()
            .map(|rule| {
                let rule = EditorRule {
                    enabled: rule.enabled,
                    mode: match rule.mode.as_str() {
                        "masks" => FilterMode::Masks,
                        "regex" => FilterMode::Regex,
                        _ => return Err(invalid("unknown editor rule pattern mode")),
                    },
                    expression: rule.expression,
                    case_sensitive: rule.case_sensitive,
                    executable: rule.executable.map(NativePath::into_path).transpose()?,
                    arguments: rule.arguments,
                    working_directory: rule
                        .working_directory
                        .map(NativePath::into_path)
                        .transpose()?,
                };
                rule.validate().map_err(invalid)?;
                Ok(rule)
            })
            .collect(),
        _ => Err(invalid("missing or unexpected editor rules")),
    }
}

impl From<EditorRule> for StoredEditorRule {
    fn from(rule: EditorRule) -> Self {
        Self {
            enabled: rule.enabled,
            mode: match rule.mode {
                FilterMode::Masks => "masks",
                FilterMode::Regex => "regex",
                FilterMode::Expression => "expression",
            }
            .into(),
            expression: rule.expression,
            case_sensitive: rule.case_sensitive,
            executable: rule.executable.map(NativePath::from_path),
            arguments: rule.arguments,
            working_directory: rule.working_directory.map(NativePath::from_path),
        }
    }
}
