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
        if !matches!(self.mode.as_str(), "masks" | "regex") {
            return Err(invalid(
                "filename colors support Masks or Regex, not metadata expressions",
            ));
        }
        let filter = StoredPanelFilter {
            enabled: true,
            mode: self.mode,
            expression: self.expression,
            case_sensitive: self.case_sensitive,
        }
        .into_filter(super::CURRENT_PREFERENCES_VERSION)?;
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
                FilterMode::Expression => "expression",
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

// Exact historical preset; customized, reordered and cleared lists stay untouched.
fn legacy_defaults() -> Vec<FilenameColorRule> {
    [
        (
            concat!(
                "*.rs;*.c;*.h;*.cpp;*.hpp;*.cc;*.cs;*.java;*.kt;*.go;*.py;*.rb;*.js;*.jsx;",
                "*.ts;*.tsx;*.swift;*.sh;*.ps1;*.sql;*.html;*.css;*.scss;*.vue;*.svelte;*.lua;",
                "*.pl;*.php;*.slint;*.toml;*.yaml;*.yml;*.json;*.xml;*.cmake;Makefile;Dockerfile"
            ),
            RgbColor(70, 155, 215),
        ),
        (
            "*.exe;*.com;*.bat;*.cmd;*.msi;*.appimage;*.run",
            RgbColor(80, 170, 100),
        ),
        (
            "*.png;*.jpg;*.jpeg;*.gif;*.webp;*.bmp;*.ico;*.tif;*.tiff;*.svg;*.avif;*.heic;*.jxl",
            RgbColor(185, 120, 210),
        ),
        (
            "*.mp4;*.mkv;*.webm;*.mov;*.avi;*.mpeg;*.mpg;*.m4v;*.wmv;*.ogv",
            RgbColor(220, 135, 65),
        ),
        (
            concat!(
                "*.pdf;*.doc;*.docx;*.odt;*.rtf;*.txt;*.md;*.rst;*.xls;*.xlsx;*.ods;",
                "*.ppt;*.pptx;*.odp;*.epub;*.csv;*.tsv"
            ),
            RgbColor(60, 165, 165),
        ),
    ]
    .into_iter()
    .map(|(expression, color)| FilenameColorRule {
        expression: expression.into(),
        style: FilenameStyle {
            foreground: Some(color),
            background: None,
        },
        ..Default::default()
    })
    .collect()
}

pub(super) fn migrate_presets(rules: Vec<FilenameColorRule>) -> Vec<FilenameColorRule> {
    let mut bright_defaults = midnight_core::default_filename_colors();
    bright_defaults.remove(2);
    bright_defaults[2].expression = "*.pdf;*.doc;*.docx;*.odt;*.rtf;*.epub".into();
    let mut slate_defaults = bright_defaults.clone();
    // Compare the complete historical list, preserving every custom rule and order.
    for index in [2, 9] {
        slate_defaults[index].style.foreground = Some(RgbColor(100, 116, 139));
    }
    let mut amber_defaults = slate_defaults.clone();
    amber_defaults[7].style.foreground = Some(RgbColor(217, 119, 6));
    if rules == legacy_defaults()
        || rules == bright_defaults
        || rules == slate_defaults
        || rules == amber_defaults
    {
        midnight_core::default_filename_colors()
    } else {
        rules
    }
}
