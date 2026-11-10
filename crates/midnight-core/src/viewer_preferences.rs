//! Persistent viewer choices. No document contents, queries or file positions.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkdownPreset {
    Compact,
    Comfortable,
    GitHub,
}

impl MarkdownPreset {
    pub fn storage_name(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Comfortable => "comfortable",
            Self::GitHub => "github",
        }
    }

    pub fn from_storage_name(value: &str) -> Option<Self> {
        match value {
            "compact" => Some(Self::Compact),
            "comfortable" => Some(Self::Comfortable),
            "github" => Some(Self::GitHub),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewerFontKind {
    Text,
    Source,
    Markdown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewerFontPreferences {
    pub family: String,
    /// Logical pixels, independently of the display's device scale.
    pub size: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewerZoomPreference {
    Fit,
    ActualPixels,
    /// 10,000 basis points means 100%; manual zoom has 0.01% stored precision.
    ScaleBasisPoints(u32),
}

impl ViewerZoomPreference {
    pub fn from_scale(scale: f64) -> Result<Self, &'static str> {
        if !scale.is_finite() || !(1.0 / 32768.0..=64.0).contains(&scale) {
            return Err("Viewer zoom is outside the supported range.");
        }
        Ok(Self::ScaleBasisPoints(
            (scale * 10_000.0).round().clamp(1.0, 640_000.0) as u32,
        ))
    }

    pub fn scale(self) -> Option<f64> {
        match self {
            Self::Fit => None,
            Self::ActualPixels => Some(1.0),
            Self::ScaleBasisPoints(value) => Some(f64::from(value) / 10_000.0),
        }
    }

    fn validate(self) -> Result<(), &'static str> {
        if matches!(self, Self::ScaleBasisPoints(value) if !(1..=640_000).contains(&value)) {
            return Err("Viewer zoom is outside the supported range.");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewerPreferences {
    pub wrap: bool,
    pub line_numbers: bool,
    pub case_sensitive: bool,
    pub text_font: ViewerFontPreferences,
    pub source_font: ViewerFontPreferences,
    pub markdown_font: ViewerFontPreferences,
    /// Text comparator family; unavailable or proportional families use a monospace fallback.
    pub comparison_font: String,
    pub markdown_preset: MarkdownPreset,
    pub image_zoom: ViewerZoomPreference,
    pub document_zoom: ViewerZoomPreference,
}

impl Default for ViewerPreferences {
    fn default() -> Self {
        Self {
            wrap: false,
            line_numbers: true,
            case_sensitive: false,
            text_font: ViewerFontPreferences {
                family: "monospace".into(),
                size: 14,
            },
            source_font: ViewerFontPreferences {
                family: "monospace".into(),
                size: 14,
            },
            markdown_font: github_font(),
            comparison_font: "Consolas".into(),
            markdown_preset: MarkdownPreset::GitHub,
            image_zoom: ViewerZoomPreference::Fit,
            document_zoom: ViewerZoomPreference::Fit,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewerPreferenceChange {
    Wrap(bool),
    LineNumbers(bool),
    CaseSensitive(bool),
    FontFamily(ViewerFontKind, String),
    FontSize(ViewerFontKind, u16),
    ComparisonFont(String),
    MarkdownPreset(MarkdownPreset),
    ImageZoom(ViewerZoomPreference),
    DocumentZoom(ViewerZoomPreference),
}

impl ViewerPreferences {
    pub fn validate(&self) -> Result<(), &'static str> {
        validate_family(&self.comparison_font)?;
        for font in [&self.text_font, &self.source_font, &self.markdown_font] {
            validate_family(&font.family)?;
            if !(8..=48).contains(&font.size) {
                return Err("Viewer font size must be between 8 and 48 logical pixels.");
            }
        }
        self.image_zoom.validate()?;
        self.document_zoom.validate()
    }

    pub fn font(&self, kind: ViewerFontKind) -> &ViewerFontPreferences {
        match kind {
            ViewerFontKind::Text => &self.text_font,
            ViewerFontKind::Source => &self.source_font,
            ViewerFontKind::Markdown => &self.markdown_font,
        }
    }

    fn font_mut(&mut self, kind: ViewerFontKind) -> &mut ViewerFontPreferences {
        match kind {
            ViewerFontKind::Text => &mut self.text_font,
            ViewerFontKind::Source => &mut self.source_font,
            ViewerFontKind::Markdown => &mut self.markdown_font,
        }
    }

    /// Apply to the latest accepted state. Invalid changes leave every field intact.
    /// Fieldwise patches avoid overwriting unrelated changes from another window.
    pub fn apply(&mut self, change: &ViewerPreferenceChange) -> Result<(), &'static str> {
        self.validate()?;
        match change {
            ViewerPreferenceChange::FontFamily(_, value)
            | ViewerPreferenceChange::ComparisonFont(value) => {
                validate_family(value)?;
            }
            ViewerPreferenceChange::FontSize(_, value) if !(8..=48).contains(value) => {
                return Err("Viewer font size must be between 8 and 48 logical pixels.");
            }
            ViewerPreferenceChange::ImageZoom(value)
            | ViewerPreferenceChange::DocumentZoom(value) => value.validate()?,
            _ => {}
        }
        let mut next = self.clone();
        match change {
            ViewerPreferenceChange::Wrap(value) => next.wrap = *value,
            ViewerPreferenceChange::LineNumbers(value) => next.line_numbers = *value,
            ViewerPreferenceChange::CaseSensitive(value) => next.case_sensitive = *value,
            ViewerPreferenceChange::FontFamily(kind, value) => {
                next.font_mut(*kind).family.clone_from(value)
            }
            ViewerPreferenceChange::FontSize(kind, value) => next.font_mut(*kind).size = *value,
            ViewerPreferenceChange::ComparisonFont(value) => next.comparison_font.clone_from(value),
            ViewerPreferenceChange::MarkdownPreset(value) => {
                next.markdown_preset = *value;
                if *value == MarkdownPreset::GitHub {
                    next.markdown_font = github_font();
                }
            }
            ViewerPreferenceChange::ImageZoom(value) => next.image_zoom = *value,
            ViewerPreferenceChange::DocumentZoom(value) => next.document_zoom = *value,
        }
        next.validate()?;
        *self = next;
        Ok(())
    }
}

fn github_font() -> ViewerFontPreferences {
    // Resolve the platform's native UI sans through the renderer's font database.
    // Applying a preset resets its typography; later explicit edits remain valid.
    ViewerFontPreferences {
        family: "system-ui".into(),
        size: 16,
    }
}

fn validate_family(value: &str) -> Result<(), &'static str> {
    if value.len() > 256 || value.trim().is_empty() || value.chars().any(char::is_control) {
        return Err("Viewer font family must contain 1–256 UTF-8 bytes without controls.");
    }
    Ok(())
}

#[cfg(test)]
#[path = "viewer_preferences_tests.rs"]
mod tests;
