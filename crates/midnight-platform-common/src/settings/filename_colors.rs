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

// Exact historical five-rule preset; only this complete list is replaced.
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

// Exact twelve-category preset with blue source code and light cool neutral Word
// documents and plain text/Markdown. The earlier eleven-rule presets, the interim
// preset and the cyan-source preset derive from it, not from today's defaults, so that
// a change of the defaults cannot alter them.
fn twelve_category_defaults() -> Vec<FilenameColorRule> {
    [
        (
            concat!(
                "*.rs;*.c;*.h;*.cpp;*.hpp;*.cc;*.cs;*.java;*.kt;*.go;*.py;*.rb;*.js;*.jsx;",
                "*.ts;*.tsx;*.swift;*.sh;*.ps1;*.sql;*.html;*.css;*.scss;*.vue;*.svelte;*.lua;",
                "*.pl;*.php;*.slint;*.toml;*.yaml;*.yml;*.json;*.xml;*.cmake;Makefile;Dockerfile"
            ),
            RgbColor(59, 130, 246),
        ),
        (
            "*.mp4;*.mkv;*.webm;*.mov;*.avi;*.mpeg;*.mpg;*.m4v;*.wmv;*.ogv",
            RgbColor(139, 92, 246),
        ),
        ("*.pdf", RgbColor(239, 90, 90)),
        ("*.doc;*.docx;*.odt;*.rtf;*.epub", RgbColor(196, 207, 224)),
        (
            "*.png;*.jpg;*.jpeg;*.gif;*.webp;*.bmp;*.ico;*.tif;*.tiff;*.svg;*.avif;*.heic;*.jxl",
            RgbColor(236, 72, 153),
        ),
        (
            "*.exe;*.com;*.bat;*.cmd;*.msi;*.appimage;*.run",
            RgbColor(249, 115, 22),
        ),
        (
            "*.xls;*.xlsx;*.xlsm;*.xlsb;*.ods;*.csv;*.tsv",
            RgbColor(34, 197, 94),
        ),
        (
            "*.mp3;*.flac;*.wav;*.ogg;*.opus;*.m4a;*.aac;*.wma;*.aiff;*.aif;*.ape;*.mid;*.midi",
            RgbColor(20, 184, 166),
        ),
        (
            "*.zip;*.7z;*.rar;*.tar;*.gz;*.bz2;*.xz;*.zst;*.tgz;*.tbz2;*.txz;*.cab;*.iso",
            RgbColor(234, 179, 8),
        ),
        (
            "*.ppt;*.pptx;*.pptm;*.pps;*.ppsx;*.odp",
            RgbColor(239, 68, 68),
        ),
        (
            "*.txt;*.md;*.markdown;*.rst;*.log;*.text;README;README.*",
            RgbColor(196, 207, 224),
        ),
        (
            "*.ttf;*.otf;*.ttc;*.woff;*.woff2;*.eot;*.fon",
            RgbColor(146, 64, 14),
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

// The cyan source code, Word documents and plain text/Markdown shared in the interim
// twelve-category preset; the preset shipped until 23 November 2026 kept it for source
// code alone.
const INTERIM_CYAN: RgbColor = RgbColor(6, 164, 199);
// Word documents and plain text/Markdown since the interim preset.
const DOCUMENT_BLUE: RgbColor = RgbColor(59, 130, 246);

pub(super) fn migrate_presets(rules: Vec<FilenameColorRule>) -> Vec<FilenameColorRule> {
    let twelve_defaults = twelve_category_defaults();
    let mut interim_defaults = twelve_defaults.clone();
    for index in [0, 3, 10] {
        interim_defaults[index].style.foreground = Some(INTERIM_CYAN);
    }
    // Shipped until 23 November 2026: cyan source code, blue documents and text.
    let mut cyan_source_defaults = interim_defaults.clone();
    for index in [3, 10] {
        cyan_source_defaults[index].style.foreground = Some(DOCUMENT_BLUE);
    }
    let mut bright_defaults = twelve_defaults.clone();
    bright_defaults.remove(2);
    bright_defaults[2].expression = "*.pdf;*.doc;*.docx;*.odt;*.rtf;*.epub".into();
    let mut slate_defaults = bright_defaults.clone();
    // Retain the existing complete-preset migration, including earlier palette changes.
    for index in [2, 9] {
        slate_defaults[index].style.foreground = Some(RgbColor(100, 116, 139));
    }
    let mut amber_defaults = slate_defaults.clone();
    amber_defaults[7].style.foreground = Some(RgbColor(217, 119, 6));
    let mut migrated = if rules == legacy_defaults()
        || rules == twelve_defaults
        || rules == interim_defaults
        || rules == cyan_source_defaults
        || rules == bright_defaults
        || rules == slate_defaults
        || rules == amber_defaults
    {
        midnight_core::default_filename_colors()
    } else {
        split_unchanged_document_rules(rules)
    };
    // Promote only exact historical DOC/TEXT signatures, including disabled rules: slate
    // gray to the light cool neutral, and the interim cyan to today's blue. This adds no
    // rows, so it also applies when the PDF split has no free slot. Source code rules
    // and any other color, mask, mode, case, target or background stay as saved.
    for rule in &mut migrated {
        if rule.mode == FilterMode::Masks
            && matches!(
                rule.expression.as_str(),
                "*.doc;*.docx;*.odt;*.rtf;*.epub"
                    | "*.txt;*.md;*.markdown;*.rst;*.log;*.text;README;README.*"
            )
            && !rule.case_sensitive
            && rule.target == FilenameColorTarget::Files
            && rule.style.background.is_none()
        {
            if rule.style.foreground == Some(RgbColor(100, 116, 139)) {
                rule.style.foreground = Some(RgbColor(196, 207, 224));
            } else if rule.style.foreground == Some(INTERIM_CYAN) {
                rule.style.foreground = Some(DOCUMENT_BLUE);
            }
        }
    }
    migrated
}

fn split_unchanged_document_rules(rules: Vec<FilenameColorRule>) -> Vec<FilenameColorRule> {
    let mut available = midnight_core::MAX_FILENAME_COLOR_RULES.saturating_sub(rules.len());
    if available == 0 {
        // Never truncate a custom rule or turn valid persisted preferences into an error.
        return rules;
    }
    let mut migrated = Vec::with_capacity(rules.len() + 1);
    for mut rule in rules {
        if available > 0 && unchanged_document_rule(&rule) {
            let mut pdf = rule.clone();
            pdf.expression = "*.pdf".into();
            pdf.style.foreground = Some(RgbColor(239, 90, 90));
            migrated.push(pdf);
            rule.expression = "*.doc;*.docx;*.odt;*.rtf;*.epub".into();
            available -= 1;
        }
        migrated.push(rule);
    }
    migrated
}

fn unchanged_document_rule(rule: &FilenameColorRule) -> bool {
    // Match only the historical combined category, never a standalone gray PDF rule.
    // Preserve enabled on both split rules so disabled categories remain disabled.
    rule.mode == FilterMode::Masks
        && rule.expression == "*.pdf;*.doc;*.docx;*.odt;*.rtf;*.epub"
        && !rule.case_sensitive
        && rule.target == FilenameColorTarget::Files
        && rule.style.background.is_none()
        && matches!(
            rule.style.foreground,
            Some(RgbColor(100, 116, 139) | RgbColor(196, 207, 224))
        )
}
