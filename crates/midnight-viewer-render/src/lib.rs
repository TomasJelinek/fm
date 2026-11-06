//! Host-owned viewer text layout. Callers run shaping and rasterization on workers.
#![forbid(unsafe_code)]

use parley::{
    FontContext, FontFamily, FontStyle, FontWeight, Layout, LayoutContext, StyleProperty,
};
use std::{borrow::Cow, ops::Range};

mod glyph_cache;
mod glyphs;
mod options;
mod raster;
mod selection;
mod source_map;

pub use options::TextOptions;
pub use raster::{Raster, Viewport};
use source_map::SourceMap;

/// A page is a bounded decoded fragment, never the complete file by default.
pub const MAX_TEXT_BYTES: usize = 256 * 1024;
const MAX_SPANS: usize = 4096;
const MAX_LINES: usize = 4096;

/// A paint-only range in the decoded UTF-8 page, sorted without overlaps.
pub struct ColorSpan {
    pub range: Range<usize>,
    pub color: [u8; 4],
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
}

impl TextRenderer {
    pub fn layout_with_options(
        &mut self,
        text: &str,
        options: &TextOptions<'_>,
        spans: &[ColorSpan],
    ) -> Result<TextPage, LayoutError> {
        validate_input(text, options.size, options.scale, spans)?;
        if options.family.len() > 256
            || options.size * options.scale > 256.0
            || !(1..=64).contains(&options.tab_width)
            || !(1..=1000).contains(&options.weight)
        {
            return Err(LayoutError::LimitExceeded);
        }
        let (shaped, source_map) = SourceMap::prepare(text, options.tab_width)?;
        let mut builder =
            self.layouts
                .ranged_builder(&mut self.fonts, &shaped, options.scale, true);
        builder.push_default(StyleProperty::FontSize(options.size));
        builder.push_default(StyleProperty::FontFamily(FontFamily::Source(
            Cow::Borrowed(options.family),
        )));
        builder.push_default(StyleProperty::FontWeight(FontWeight::new(f32::from(
            options.weight,
        ))));
        builder.push_default(StyleProperty::FontStyle(if options.italic {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        }));
        builder.push_default(StyleProperty::Brush(options.foreground));
        for span in spans {
            let range =
                source_map.to_layout(span.range.start)..source_map.to_layout(span.range.end);
            if !range.is_empty() {
                builder.push(StyleProperty::Brush(span.color), range);
            }
        }
        let mut layout = builder.build(&shaped);
        layout.break_all_lines(None);
        Ok(TextPage {
            text: text.into(),
            layout,
            source_map,
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
        self.layout.len()
    }

    pub fn width(&self) -> f32 {
        self.layout.width()
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
