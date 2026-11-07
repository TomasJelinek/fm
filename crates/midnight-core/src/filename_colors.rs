//! Bounded filename color preferences and immutable worker presentation results.
use crate::{Entry, EntryId, FilterMode, PaneId, PanelFilter};
use std::{collections::HashMap, sync::Arc};

pub const MAX_FILENAME_COLOR_RULES: usize = 32;

/// Starting presets; presentation derives contrast for each theme and selection state.
/// Only files match, preserving the theme's distinct directory appearance.
pub fn default_filename_colors() -> Vec<FilenameColorRule> {
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RgbColor(pub u8, pub u8, pub u8);

impl RgbColor {
    pub fn parse(value: &str) -> Result<Self, &'static str> {
        let bytes = value.as_bytes();
        if bytes.len() != 7 || bytes[0] != b'#' || !bytes[1..].iter().all(u8::is_ascii_hexdigit) {
            return Err("Use a color in #RRGGBB format.");
        }
        let number = u32::from_str_radix(&value[1..], 16).map_err(|_| "Invalid color.")?;
        Ok(Self(
            (number >> 16) as u8,
            (number >> 8) as u8,
            number as u8,
        ))
    }

    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FilenameColorTarget {
    #[default]
    Files,
    Folders,
    All,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FilenameStyle {
    pub foreground: Option<RgbColor>,
    pub background: Option<RgbColor>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilenameColorRule {
    pub enabled: bool,
    pub mode: FilterMode,
    pub expression: String,
    pub case_sensitive: bool,
    pub target: FilenameColorTarget,
    pub style: FilenameStyle,
}

impl Default for FilenameColorRule {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: FilterMode::Masks,
            expression: "*.*".into(),
            case_sensitive: false,
            target: FilenameColorTarget::Files,
            style: FilenameStyle::default(),
        }
    }
}

impl FilenameColorRule {
    /// Disabled drafts are validated too, so enabling cannot revive bad syntax.
    pub fn filter(&self) -> PanelFilter {
        PanelFilter {
            enabled: true,
            mode: self.mode,
            expression: self.expression.clone(),
            case_sensitive: self.case_sensitive,
        }
    }
}

pub fn validate_filename_colors(rules: &[FilenameColorRule]) -> Result<(), &'static str> {
    if rules.len() > MAX_FILENAME_COLOR_RULES {
        return Err("At most 32 filename color rules are supported.");
    }
    for rule in rules {
        if rule.mode == FilterMode::Expression {
            return Err(
                "Filename colors support Masks or Regex; metadata expressions are panel filters.",
            );
        }
        rule.filter().validate_bounds()?;
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct FilenameColorsRequest {
    pub id: u64,
    pub pane: PaneId,
    pub source: Arc<[Entry]>,
    pub rules: Arc<[FilenameColorRule]>,
}

#[derive(Clone, Debug)]
pub struct FilenameColorsResult {
    pub request: FilenameColorsRequest,
    /// IDs are checked for uniqueness before publication; source Arc is authoritative.
    pub styles: HashMap<EntryId, FilenameStyle>,
}

#[derive(Clone, Debug)]
pub struct FilenameColorsValidation {
    pub id: u64,
    pub rules: Vec<FilenameColorRule>,
}
