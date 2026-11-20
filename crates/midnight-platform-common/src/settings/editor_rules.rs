//! Version 30 editor associations, with the previous editor retained as fallback.
//! A rule whose program or folder belongs to another system is kept inactive.
use super::{NativePath, invalid, native_path::Foreign};
use midnight_core::{EditorRule, FilterMode, ForeignSetting, ForeignSettingKind};
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
) -> io::Result<(Vec<EditorRule>, Vec<ForeignSetting>)> {
    match (version, rules) {
        (1..=29, None) => Ok((Vec::new(), Vec::new())),
        (30.., Some(stored)) => {
            let (mut rules, mut foreign) = (Vec::new(), Vec::new());
            for (position, rule) in stored.into_iter().enumerate() {
                match rule.foreign()? {
                    Some(found) => foreign.push(rule.into_foreign(position, found)?),
                    None => rules.push(rule.into_rule()?),
                }
            }
            Ok((rules, foreign))
        }
        _ => Err(invalid("missing or unexpected editor rules")),
    }
}

/// Return kept rules to their stored positions; rule order decides which editor wins.
pub(super) fn merge(
    mut rules: Vec<StoredEditorRule>,
    foreign: &[ForeignSetting],
) -> io::Result<Vec<StoredEditorRule>> {
    for (position, stored) in super::foreign::positioned(foreign, |kind| match kind {
        ForeignSettingKind::EditorRule { position, .. } => Some(*position),
        _ => None,
    }) {
        let rule = serde_json::from_str(stored).map_err(invalid)?;
        rules.insert(position.min(rules.len()), rule);
    }
    Ok(rules)
}

impl StoredEditorRule {
    fn into_rule(self) -> io::Result<EditorRule> {
        let rule = EditorRule {
            enabled: self.enabled,
            mode: match self.mode.as_str() {
                "masks" => FilterMode::Masks,
                "regex" => FilterMode::Regex,
                _ => return Err(invalid("unknown editor rule pattern mode")),
            },
            expression: self.expression,
            case_sensitive: self.case_sensitive,
            executable: self.executable.map(NativePath::into_path).transpose()?,
            arguments: self.arguments,
            working_directory: self
                .working_directory
                .map(NativePath::into_path)
                .transpose()?,
        };
        rule.validate().map_err(invalid)?;
        Ok(rule)
    }

    fn foreign(&self) -> io::Result<Option<Foreign>> {
        for path in [&self.executable, &self.working_directory]
            .into_iter()
            .flatten()
        {
            if let Some(foreign) = path.foreign()? {
                return Ok(Some(foreign));
            }
        }
        Ok(None)
    }

    /// Keep this record verbatim after validating everything except its foreign paths.
    fn into_foreign(
        mut self,
        position: usize,
        (system, location): Foreign,
    ) -> io::Result<ForeignSetting> {
        let stored = serde_json::to_string(&self).map_err(invalid)?.into();
        for path in [&mut self.executable, &mut self.working_directory]
            .into_iter()
            .flatten()
        {
            *path = path.validation_copy()?;
        }
        let rule = self.into_rule()?;
        Ok(ForeignSetting {
            kind: ForeignSettingKind::EditorRule {
                position,
                pattern: rule.expression,
            },
            system,
            location,
            stored,
        })
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
