//! WCAG 2 contrast, and readable colors for selected glyphs. A selected glyph takes
//! the selection's text color, or keeps its own color when the selection has none.
//! Either color keeps its hue and saturation: only its HSL lightness moves, and only
//! as far as the minimum contrast against the painted selection background requires.
use crate::raster::{blend_pixel, premultiply};
use std::collections::HashMap;

/// WCAG 2 level AA contrast ratio for normal-size text.
pub(crate) const MINIMUM: f64 = 4.5;
/// A page has at most a few thousand color spans; more distinct brushes than
/// this restart the cache instead of growing it.
const MAX_BRUSHES: usize = 8192;
/// Bisection steps of HSL lightness: far finer than one 8-bit channel step.
const STEPS: usize = 20;

/// WCAG 2 contrast ratio of two opaque colors. Alpha channels are ignored.
pub(crate) fn ratio(a: [u8; 4], b: [u8; 4]) -> f64 {
    between(luminance([a[0], a[1], a[2]]), luminance([b[0], b[1], b[2]]))
}

/// The contrast that content colors keep against a page of `foreground` text on
/// `background`: 5:1, a margin over [`MINIMUM`], and 7:1 where the body text itself
/// reaches 15:1, as in the Contrast and High Contrast themes. The viewer's syntax colors
/// follow the same rule (`midnight_core::content_minimum`).
pub(crate) fn content_minimum(foreground: [u8; 4], background: [u8; 4]) -> f64 {
    if ratio(foreground, background) >= 15.0 {
        7.0
    } else {
        5.0
    }
}

/// Brushes of selected glyphs for one selection background and text color. Each
/// source brush is adjusted once and reused until the selection, its text color or
/// the viewport background changes; painting only looks them up.
#[derive(Default)]
pub(crate) struct SelectionColors {
    /// The selection color as painted over the viewport background, opaque.
    background: [u8; 3],
    /// The selection's text color; `None` keeps each glyph's own color.
    text: Option<[u8; 4]>,
    brushes: HashMap<[u8; 4], [u8; 4]>,
}

impl SelectionColors {
    /// Adjust brushes for `selection` painted over `background` and its `text` color
    /// (straight RGBA; a fully transparent `text` keeps each glyph's own color).
    /// False when the selection is fully transparent: selected text then shows
    /// over the unchanged background and keeps its colors.
    pub(crate) fn prepare(
        &mut self,
        selection: [u8; 4],
        background: [u8; 4],
        text: [u8; 4],
    ) -> bool {
        if selection[3] == 0 {
            return false;
        }
        // Exactly the pixel that the selection fill paints over the cleared
        // viewport. Its alpha is at least the selection's, so never zero.
        let mut pixel = premultiply(background);
        blend_pixel(&mut pixel, premultiply(selection));
        let alpha = u16::from(pixel[3]);
        let opaque = std::array::from_fn(|channel| {
            ((u16::from(pixel[channel]) * 255 + alpha / 2) / alpha).min(255) as u8
        });
        let text = (text[3] != 0).then_some(text);
        if opaque != self.background || text != self.text {
            self.background = opaque;
            self.text = text;
            self.brushes.clear();
        }
        true
    }

    /// The brush that a selected glyph uses instead of `brush`: the selection's text
    /// color at the glyph's own opacity, or `brush` itself without one, made readable.
    /// An invisible glyph stays invisible.
    pub(crate) fn brush(&mut self, brush: [u8; 4]) -> [u8; 4] {
        if let Some(readable) = self.brushes.get(&brush) {
            return *readable;
        }
        if self.brushes.len() >= MAX_BRUSHES {
            self.brushes.clear();
        }
        let source = match self.text {
            Some([red, green, blue, alpha]) => {
                let alpha = (u16::from(alpha) * u16::from(brush[3]) + 127) / 255;
                [red, green, blue, alpha as u8]
            }
            None => brush,
        };
        let readable = readable_brush(source, self.background);
        self.brushes.insert(brush, readable);
        readable
    }
}

/// `brush` itself when it reaches [`MINIMUM`] contrast over `background`;
/// otherwise the color of its hue and saturation with the nearest HSL lightness
/// that does, lighter or darker; failing both, the better of black and white.
/// A translucent brush is judged as painted over the background; alpha is kept.
fn readable_brush(brush: [u8; 4], background: [u8; 3]) -> [u8; 4] {
    let [red, green, blue, alpha] = brush;
    let backdrop = luminance(background);
    let contrast = |rgb: [u8; 3]| {
        let shown = std::array::from_fn(|channel| mix(rgb[channel], background[channel], alpha));
        between(luminance(shown), backdrop)
    };
    let reaches = |rgb| contrast(rgb) >= MINIMUM;
    let original = [red, green, blue];
    if alpha == 0 || reaches(original) {
        return brush;
    }
    let [hue, saturation, lightness] = hsl(original);
    let color = |lightness| rgb(hue, saturation, lightness);
    // Luminance never decreases with lightness. From an unreadable start, each
    // direction therefore has one boundary, found by bisection towards white
    // (1) or black (0) when that end itself is readable.
    let nearest = |end: f64| {
        reaches(color(end)).then(|| {
            let (mut unreadable, mut found) = (lightness, end);
            for _ in 0..STEPS {
                let middle = (unreadable + found) / 2.0;
                if reaches(color(middle)) {
                    found = middle;
                } else {
                    unreadable = middle;
                }
            }
            found
        })
    };
    let chosen = match (nearest(1.0), nearest(0.0)) {
        (Some(lighter), Some(darker)) if lighter - lightness <= lightness - darker => lighter,
        (_, Some(darker)) => darker,
        (Some(lighter), None) => lighter,
        (None, None) if contrast([255; 3]) >= contrast([0; 3]) => 1.0,
        (None, None) => 0.0,
    };
    let [red, green, blue] = color(chosen);
    [red, green, blue, alpha]
}

/// `top` painted with `alpha` over an opaque `bottom` channel.
fn mix(top: u8, bottom: u8, alpha: u8) -> u8 {
    let alpha = u16::from(alpha);
    ((u16::from(top) * alpha + u16::from(bottom) * (255 - alpha) + 127) / 255) as u8
}

/// WCAG 2 relative luminance of an sRGB color.
fn luminance(rgb: [u8; 3]) -> f64 {
    let linear = |channel: u8| {
        let value = f64::from(channel) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(rgb[0]) + 0.7152 * linear(rgb[1]) + 0.0722 * linear(rgb[2])
}

fn between(a: f64, b: f64) -> f64 {
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// Hue in sixths of a turn, `[0, 6)`, then saturation and lightness in `[0, 1]`.
fn hsl(rgb: [u8; 3]) -> [f64; 3] {
    let [red, green, blue] = rgb.map(|channel| f64::from(channel) / 255.0);
    let max = red.max(green).max(blue);
    let min = red.min(green).min(blue);
    let lightness = (max + min) / 2.0;
    let chroma = max - min;
    if chroma == 0.0 {
        return [0.0, 0.0, lightness];
    }
    let hue = if max == red {
        ((green - blue) / chroma).rem_euclid(6.0)
    } else if max == green {
        (blue - red) / chroma + 2.0
    } else {
        (red - green) / chroma + 4.0
    };
    // Chroma is positive, so lightness is strictly between 0 and 1.
    let saturation = chroma / (1.0 - (2.0 * lightness - 1.0).abs());
    [hue, saturation, lightness]
}

fn rgb(hue: f64, saturation: f64, lightness: f64) -> [u8; 3] {
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let second = chroma * (1.0 - (hue.rem_euclid(2.0) - 1.0).abs());
    let [red, green, blue] = match hue as u8 {
        0 => [chroma, second, 0.0],
        1 => [second, chroma, 0.0],
        2 => [0.0, chroma, second],
        3 => [0.0, second, chroma],
        4 => [second, 0.0, chroma],
        _ => [chroma, 0.0, second],
    };
    let base = lightness - chroma / 2.0;
    [red, green, blue].map(|channel| ((channel + base) * 255.0).round().clamp(0.0, 255.0) as u8)
}

#[cfg(test)]
#[path = "contrast_tests.rs"]
mod tests;
