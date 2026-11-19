//! Strict schema-33 comparator DTO; native path identity stays lossless.
use super::{NativePath, invalid, present};
use midnight_core::{
    FileComparatorPreferences, FileComparatorProgram, FileComparatorRule, FileComparatorTarget,
    FilterMode,
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

pub(super) fn decode(
    version: u32,
    stored: Option<StoredFileComparators>,
) -> io::Result<FileComparatorPreferences> {
    match (version, stored) {
        (1..=32, None) => Ok(FileComparatorPreferences::default()),
        (33.., Some(stored)) => {
            let rules = stored
                .rules
                .into_iter()
                .map(|rule| {
                    Ok(FileComparatorRule {
                        enabled: rule.enabled,
                        mode: match rule.mode.as_str() {
                            "masks" => FilterMode::Masks,
                            "regex" => FilterMode::Regex,
                            _ => return Err(invalid("unknown comparator rule pattern mode")),
                        },
                        expression: rule.expression,
                        case_sensitive: rule.case_sensitive,
                        target: match rule.target {
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
                                working_directory: working_directory
                                    .map(NativePath::into_path)
                                    .transpose()?,
                            }),
                        },
                    })
                })
                .collect::<io::Result<Vec<_>>>()?;
            let preferences = FileComparatorPreferences { rules };
            preferences.validate().map_err(invalid)?;
            Ok(preferences)
        }
        _ => Err(invalid("missing or unexpected file comparator preferences")),
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
