//! Strict schema-33 comparator DTO; native path identity stays lossless.
//! A rule whose program or folder belongs to another system is kept inactive.
use super::{NativePath, invalid, native_path::Foreign, present};
use midnight_core::{
    FileComparatorPreferences, FileComparatorProgram, FileComparatorRule, FileComparatorTarget,
    FilterMode, ForeignSetting, ForeignSettingKind,
};
use serde::{Deserialize, Serialize};
use std::io;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredFileComparators {
    rules: Vec<StoredRule>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredRule {
    enabled: bool,
    mode: String,
    expression: String,
    case_sensitive: bool,
    target: StoredTarget,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StoredTarget {
    Automatic {},
    BuiltInText {},
    BuiltInImages {},
    External {
        executable: NativePath,
        arguments: Vec<String>,
        #[serde(
            default,
            deserialize_with = "present",
            skip_serializing_if = "Option::is_none"
        )]
        working_directory: Option<NativePath>,
    },
}

/// Rules whose program or folder belongs to another system are kept inactive.
pub(super) fn decode(
    version: u32,
    stored: Option<StoredFileComparators>,
) -> io::Result<(FileComparatorPreferences, Vec<ForeignSetting>)> {
    match (version, stored) {
        (1..=32, None) => Ok((FileComparatorPreferences::default(), Vec::new())),
        (33.., Some(stored)) => {
            let (mut rules, mut foreign) = (Vec::new(), Vec::new());
            for (position, rule) in stored.rules.into_iter().enumerate() {
                match rule.foreign()? {
                    Some(found) => foreign.push(rule.into_foreign(position, found)?),
                    None => rules.push(rule.into_rule()?),
                }
            }
            let preferences = FileComparatorPreferences { rules };
            preferences.validate().map_err(invalid)?;
            Ok((preferences, foreign))
        }
        _ => Err(invalid("missing or unexpected file comparator preferences")),
    }
}

/// Return kept rules to their stored positions; rule order decides the comparator.
pub(super) fn merge(
    mut stored: StoredFileComparators,
    foreign: &[ForeignSetting],
) -> io::Result<StoredFileComparators> {
    for (position, record) in super::foreign::positioned(foreign, |kind| match kind {
        ForeignSettingKind::FileComparator { position, .. } => Some(*position),
        _ => None,
    }) {
        let rule = serde_json::from_str(record).map_err(invalid)?;
        stored.rules.insert(position.min(stored.rules.len()), rule);
    }
    Ok(stored)
}

impl StoredRule {
    fn into_rule(self) -> io::Result<FileComparatorRule> {
        Ok(FileComparatorRule {
            enabled: self.enabled,
            mode: match self.mode.as_str() {
                "masks" => FilterMode::Masks,
                "regex" => FilterMode::Regex,
                _ => return Err(invalid("unknown comparator rule pattern mode")),
            },
            expression: self.expression,
            case_sensitive: self.case_sensitive,
            target: match self.target {
                StoredTarget::Automatic {} => FileComparatorTarget::Automatic,
                StoredTarget::BuiltInText {} => FileComparatorTarget::BuiltInText,
                StoredTarget::BuiltInImages {} => FileComparatorTarget::BuiltInImages,
                StoredTarget::External {
                    executable,
                    arguments,
                    working_directory,
                } => FileComparatorTarget::External(FileComparatorProgram {
                    executable: executable.into_path()?,
                    arguments,
                    working_directory: working_directory.map(NativePath::into_path).transpose()?,
                }),
            },
        })
    }

    fn foreign(&self) -> io::Result<Option<Foreign>> {
        let StoredTarget::External {
            executable,
            working_directory,
            ..
        } = &self.target
        else {
            return Ok(None);
        };
        for path in std::iter::once(executable).chain(working_directory) {
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
        if let StoredTarget::External {
            executable,
            working_directory,
            ..
        } = &mut self.target
        {
            *executable = executable.validation_copy()?;
            if let Some(directory) = working_directory {
                *directory = directory.validation_copy()?;
            }
        }
        let rule = self.into_rule()?;
        rule.validate().map_err(invalid)?;
        Ok(ForeignSetting {
            kind: ForeignSettingKind::FileComparator {
                position,
                pattern: rule.expression,
            },
            system,
            location,
            stored,
        })
    }
}

impl From<FileComparatorPreferences> for StoredFileComparators {
    fn from(preferences: FileComparatorPreferences) -> Self {
        Self {
            rules: preferences
                .rules
                .into_iter()
                .map(|rule| StoredRule {
                    enabled: rule.enabled,
                    mode: match rule.mode {
                        FilterMode::Masks => "masks",
                        FilterMode::Regex => "regex",
                        FilterMode::Expression => "expression",
                    }
                    .into(),
                    expression: rule.expression,
                    case_sensitive: rule.case_sensitive,
                    target: match rule.target {
                        FileComparatorTarget::Automatic => StoredTarget::Automatic {},
                        FileComparatorTarget::BuiltInText => StoredTarget::BuiltInText {},
                        FileComparatorTarget::BuiltInImages => StoredTarget::BuiltInImages {},
                        FileComparatorTarget::External(program) => StoredTarget::External {
                            executable: NativePath::from_path(program.executable),
                            arguments: program.arguments,
                            working_directory: program.working_directory.map(NativePath::from_path),
                        },
                    },
                })
                .collect(),
        }
    }
}
