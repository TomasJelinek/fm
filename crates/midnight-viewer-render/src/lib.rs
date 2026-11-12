//! Host-owned viewer text layout. Callers run shaping and rasterization on workers.
#![forbid(unsafe_code)]

use parley::{
    FontContext, FontFamily, FontStyle, FontWeight, Layout, LayoutContext, OverflowWrap,
    StyleProperty,
};
use std::{borrow::Cow, ops::Range};

mod document;
mod fonts;
pub use document::{DocumentBlock, DocumentKind, DocumentStyle};
mod glyph_cache;
pub use fonts::{installed_families, is_generic_family, monospace_families};
mod glyphs;
mod options;
mod raster;
mod scroll_anchors;
mod selection;
mod source_map;
mod tables;

pub use options::TextOptions;
pub use raster::{Raster, Viewport};
use source_map::SourceMap;
pub use tables::{CellAlignment, TableCell, TableRow};

/// A page is a bounded decoded fragment, never the complete file by default.
pub const MAX_TEXT_BYTES: usize = 256 * 1024;
const MAX_SPANS: usize = 4096;
// Splitting the API's independent span/decoration ranges preserves both styles.
const MAX_STYLE_SPANS: usize = 4 * MAX_SPANS + 512;
const MAX_LINES: usize = 4096;

/// A paint-only range in the decoded UTF-8 page, sorted without overlaps.
#[derive(Clone)]
pub struct ColorSpan {
    pub range: Range<usize>,
    pub color: [u8; 4],
}

/// Host-selected typography for a bounded semantic range (for example a heading).
#[derive(Clone)]
pub struct StyleSpan {
    pub range: Range<usize>,
    pub weight: u16,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub monospace: bool,
    pub size: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum LayoutError {
    LimitExceeded,
    InvalidGeometry,
    InvalidRange,
    InvalidFont,
}

#[derive(Default)]
pub struct TextRenderer {
    fonts: FontContext,
    layouts: LayoutContext<[u8; 4]>,
    glyphs: glyphs::GlyphPainter,
}

pub struct TextPage {
    text: String,
    layout: Layout<[u8; 4]>,
    source_map: SourceMap,
    composite: Option<tables::Composite>,
}

impl TextRenderer {
    pub fn layout_with_options(
        &mut self,
        text: &str,
        options: &TextOptions<'_>,
        spans: &[ColorSpan],
    ) -> Result<TextPage, LayoutError> {
        self.layout_with_styles(text, options, spans, &[])
    }

    pub fn layout_with_styles(
        &mut self,
        text: &str,
        options: &TextOptions<'_>,
        spans: &[ColorSpan],
        styles: &[StyleSpan],
    ) -> Result<TextPage, LayoutError> {
        validate_input(text, options.size, options.scale, spans)?;
        if options
            .wrap_width
            .is_some_and(|width| !width.is_finite() || width <= 0.0)
        {
            return Err(LayoutError::InvalidGeometry);
        }
        if options
            .line_height
            .is_some_and(|height| !height.is_finite() || !(1.0..=3.0).contains(&height))
        {
            return Err(LayoutError::InvalidGeometry);
        }
        validate_styles(text, options.scale, styles)?;
        if options.family.len() > 256
            || options.size * options.scale > 256.0
            || !(1..=64).contains(&options.tab_width)
            || !(1..=1000).contains(&options.weight)
        {
            return Err(LayoutError::LimitExceeded);
        }
        let (shaped, source_map) = SourceMap::prepare(text, options.tab_width)?;
        let mut builder = self
            .layouts
            // Page-local baseline rounding changes the spacing pattern when
            // an overlapping fragment starts at another fractional line.
            .ranged_builder(&mut self.fonts, &shaped, options.scale, false);
        builder.push_default(StyleProperty::FontSize(options.size));
        if let Some(height) = options.line_height {
            builder.push_default(StyleProperty::LineHeight(
                parley::style::LineHeight::FontSizeRelative(height),
            ));
        }
        builder.push_default(StyleProperty::FontFamily(if options.literal_family {
            FontFamily::named(options.family)
        } else {
            FontFamily::Source(Cow::Borrowed(options.family))
        }));
        builder.push_default(StyleProperty::FontWeight(FontWeight::new(f32::from(
            options.weight,
        ))));
        builder.push_default(StyleProperty::FontStyle(if options.italic {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        }));
        builder.push_default(StyleProperty::Brush(options.foreground));
        if options.tabular_numbers {
            builder.push_default(StyleProperty::FontFeatures("\"tnum\"".into()));
        }
        // Prefer word boundaries; an oversized word must still fit the viewport.
        if options.wrap_width.is_some() {
            builder.push_default(StyleProperty::OverflowWrap(OverflowWrap::Anywhere));
        }
        for style in styles {
            let range =
                source_map.to_layout(style.range.start)..source_map.to_layout(style.range.end);
            builder.push(StyleProperty::FontSize(style.size), range.clone());
            builder.push(
                StyleProperty::FontWeight(FontWeight::new(f32::from(style.weight))),
                range.clone(),
            );
            builder.push(
                StyleProperty::FontStyle(if style.italic {
                    FontStyle::Italic
                } else {
                    FontStyle::Normal
                }),
                range.clone(),
            );
            if style.monospace {
                builder.push(
                    StyleProperty::FontFamily(FontFamily::Source(Cow::Borrowed("monospace"))),
                    range.clone(),
                );
            }
            builder.push(
                StyleProperty::Strikethrough(style.strikethrough),
                range.clone(),
            );
            builder.push(StyleProperty::Underline(style.underline), range);
        }
        for span in spans {
            let range =
                source_map.to_layout(span.range.start)..source_map.to_layout(span.range.end);
            if !range.is_empty() {
                builder.push(StyleProperty::Brush(span.color), range);
            }
        }
        let mut layout = builder.build(&shaped);
        layout.break_all_lines(options.wrap_width);
        Ok(TextPage {
            text: text.into(),
            layout,
            source_map,
            composite: None,
        })
    }

    pub fn layout_styled(
        &mut self,
        text: &str,
        size: f32,
        scale: f32,
        spans: &[ColorSpan],
    ) -> Result<TextPage, LayoutError> {
        self.layout_with_options(
            text,
            &TextOptions {
                size,
                scale,
                ..TextOptions::default()
            },
            spans,
        )
    }

    pub fn layout(&mut self, text: &str, size: f32, scale: f32) -> Result<TextPage, LayoutError> {
        self.layout_styled(text, size, scale, &[])
    }
}

impl TextPage {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn line_count(&self) -> usize {
        self.composite.as_ref().map_or_else(
            || self.layout.len(),
            |grid| grid.parts.iter().map(|part| part.page.line_count()).sum(),
        )
    }

    pub fn width(&self) -> f32 {
        self.composite
            .as_ref()
            .map_or_else(|| self.layout.width(), |grid| grid.width)
    }

    pub fn height(&self) -> f32 {
        self.composite
            .as_ref()
            .map_or_else(|| self.layout.height(), |grid| grid.height)
    }
}

fn validate_input(
    text: &str,
    size: f32,
    scale: f32,
    spans: &[ColorSpan],
) -> Result<(), LayoutError> {
    if text.len() > MAX_TEXT_BYTES || spans.len() > MAX_SPANS || hard_line_count(text) > MAX_LINES {
        return Err(LayoutError::LimitExceeded);
    }
    if !size.is_finite()
        || !scale.is_finite()
        || !(1.0..=512.0).contains(&size)
        || !(0.25..=8.0).contains(&scale)
    {
        return Err(LayoutError::InvalidGeometry);
    }
    let mut previous_end = 0;
    for span in spans {
        if span.range.start < previous_end || text.get(span.range.clone()).is_none() {
            return Err(LayoutError::InvalidRange);
        }
        previous_end = span.range.end;
    }
    Ok(())
}

fn validate_styles(text: &str, scale: f32, styles: &[StyleSpan]) -> Result<(), LayoutError> {
    if styles.len() > MAX_STYLE_SPANS {
        return Err(LayoutError::LimitExceeded);
    }
    for style in styles {
        if text.get(style.range.clone()).is_none() || style.range.is_empty() {
            return Err(LayoutError::InvalidRange);
        }
        if !style.size.is_finite()
            || !(1.0..=256.0).contains(&(style.size * scale))
            || !(1..=1000).contains(&style.weight)
        {
            return Err(LayoutError::LimitExceeded);
        }
    }
    Ok(())
}

fn hard_line_count(text: &str) -> usize {
    let mut count = 1;
    let mut after_cr = false;
    for character in text.chars() {
        let is_break = is_line_break(character);
        if is_break && !(character == '\n' && after_cr) {
            count += 1;
        }
        after_cr = character == '\r';
    }
    count
}

fn is_line_break(character: char) -> bool {
    matches!(
        character,
        '\r' | '\n' | '\u{000b}' | '\u{000c}' | '\u{0085}' | '\u{2028}' | '\u{2029}'
    )
}

impl std::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::LimitExceeded => "viewer layout exceeds its page budget",
            Self::InvalidGeometry => "viewer geometry is outside supported finite bounds",
            Self::InvalidRange => "viewer range is invalid, overlapping or splits UTF-8",
            Self::InvalidFont => "viewer font data or glyph is unsupported",
        })
    }
}

impl std::error::Error for LayoutError {}
