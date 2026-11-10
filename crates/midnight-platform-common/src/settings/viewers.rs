//! Bounded viewer choices; schema22 adds the independent text comparator font.
use midnight_core::{
    MarkdownPreset, ViewerFontPreferences, ViewerPreferences, ViewerZoomPreference,
};
use serde::{Deserialize, Serialize};
use std::io;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredViewerPreferences {
    wrap: bool,
    line_numbers: bool,
    case_sensitive: bool,
    text_font: StoredFont,
    source_font: StoredFont,
    markdown_font: StoredFont,
    #[serde(
        default,
        deserialize_with = "super::present",
        skip_serializing_if = "Option::is_none"
    )]
    comparison_font: Option<String>,
    markdown_preset: String,
    image_zoom: StoredZoom,
    document_zoom: StoredZoom,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredFont {
    family: String,
    size: u16,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "kebab-case", deny_unknown_fields)]
enum StoredZoom {
    Fit,
    ActualPixels,
    Scale { basis_points: u32 },
}

impl From<&ViewerFontPreferences> for StoredFont {
    fn from(value: &ViewerFontPreferences) -> Self {
        Self {
            family: value.family.clone(),
            size: value.size,
        }
    }
}
impl From<StoredFont> for ViewerFontPreferences {
    fn from(value: StoredFont) -> Self {
        Self {
            family: value.family,
            size: value.size,
        }
    }
}
impl From<ViewerZoomPreference> for StoredZoom {
    fn from(value: ViewerZoomPreference) -> Self {
        match value {
            ViewerZoomPreference::Fit => Self::Fit,
            ViewerZoomPreference::ActualPixels => Self::ActualPixels,
            ViewerZoomPreference::ScaleBasisPoints(basis_points) => Self::Scale { basis_points },
        }
    }
}
impl From<StoredZoom> for ViewerZoomPreference {
    fn from(value: StoredZoom) -> Self {
        match value {
            StoredZoom::Fit => Self::Fit,
            StoredZoom::ActualPixels => Self::ActualPixels,
            StoredZoom::Scale { basis_points } => Self::ScaleBasisPoints(basis_points),
        }
    }
}

impl TryFrom<&ViewerPreferences> for StoredViewerPreferences {
    type Error = io::Error;
    fn try_from(value: &ViewerPreferences) -> io::Result<Self> {
        value.validate().map_err(invalid)?;
        Ok(Self {
            wrap: value.wrap,
            line_numbers: value.line_numbers,
            case_sensitive: value.case_sensitive,
            text_font: (&value.text_font).into(),
            source_font: (&value.source_font).into(),
            markdown_font: (&value.markdown_font).into(),
            comparison_font: (value.comparison_font
                != ViewerPreferences::default().comparison_font)
                .then(|| value.comparison_font.clone()),
            markdown_preset: value.markdown_preset.storage_name().into(),
            image_zoom: value.image_zoom.into(),
            document_zoom: value.document_zoom.into(),
        })
    }
}

impl TryFrom<StoredViewerPreferences> for ViewerPreferences {
    type Error = io::Error;
    fn try_from(value: StoredViewerPreferences) -> io::Result<Self> {
        let prefs = Self {
            wrap: value.wrap,
            line_numbers: value.line_numbers,
            case_sensitive: value.case_sensitive,
            text_font: value.text_font.into(),
            source_font: value.source_font.into(),
            markdown_font: value.markdown_font.into(),
            comparison_font: value
                .comparison_font
                .unwrap_or_else(|| ViewerPreferences::default().comparison_font),
            markdown_preset: MarkdownPreset::from_storage_name(&value.markdown_preset)
                .ok_or_else(|| invalid("Invalid Markdown rendering preset."))?,
            image_zoom: value.image_zoom.into(),
            document_zoom: value.document_zoom.into(),
        };
        prefs.validate().map_err(invalid)?;
        Ok(prefs)
    }
}

impl StoredViewerPreferences {
    pub(super) fn validate_version(&self, version: u32) -> io::Result<()> {
        // Version 23 can be required by a panel split with the default viewer font.
        if (version < 22 && self.comparison_font.is_some())
            || (version == 22 && self.comparison_font.is_none())
        {
            return Err(invalid("Comparison font requires preferences version 22."));
        }
        Ok(())
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use midnight_core::{ViewerFontKind, ViewerPreferenceChange};

    #[test]
    fn all_choices_roundtrip_without_content_or_floating_point_settings() {
        let mut prefs = ViewerPreferences::default();
        for change in [
            ViewerPreferenceChange::Wrap(true),
            ViewerPreferenceChange::LineNumbers(false),
            ViewerPreferenceChange::CaseSensitive(true),
            ViewerPreferenceChange::FontFamily(ViewerFontKind::Source, "Cascadia Code 世界".into()),
            ViewerPreferenceChange::FontSize(ViewerFontKind::Source, 19),
            ViewerPreferenceChange::MarkdownPreset(MarkdownPreset::Comfortable),
            ViewerPreferenceChange::ImageZoom(ViewerZoomPreference::ScaleBasisPoints(15_625)),
            ViewerPreferenceChange::DocumentZoom(ViewerZoomPreference::ActualPixels),
        ] {
            prefs.apply(&change).unwrap();
        }
        let bytes =
            serde_json::to_vec(&StoredViewerPreferences::try_from(&prefs).unwrap()).unwrap();
        let stored = serde_json::from_slice::<StoredViewerPreferences>(&bytes).unwrap();
        assert_eq!(ViewerPreferences::try_from(stored).unwrap(), prefs);
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("15625") && !text.contains("1.5625"));
        assert!(!text.contains("query") && !text.contains("position") && !text.contains("content"));
    }

    #[test]
    fn hostile_stored_values_are_refused_on_both_encoding_and_decoding() {
        let mutations: [fn(&mut StoredViewerPreferences); 4] = [
            |value: &mut StoredViewerPreferences| value.source_font.family = "\0bad".into(),
            |value: &mut StoredViewerPreferences| value.markdown_font.size = 0,
            |value: &mut StoredViewerPreferences| value.markdown_preset = "unknown".into(),
            |value: &mut StoredViewerPreferences| {
                value.image_zoom = StoredZoom::Scale { basis_points: 0 }
            },
        ];
        for mutate in mutations {
            let mut stored =
                StoredViewerPreferences::try_from(&ViewerPreferences::default()).unwrap();
            mutate(&mut stored);
            assert!(ViewerPreferences::try_from(stored).is_err());
        }
        let mut prefs = ViewerPreferences::default();
        prefs.source_font.size = 1000;
        assert!(StoredViewerPreferences::try_from(&prefs).is_err());
    }
}
