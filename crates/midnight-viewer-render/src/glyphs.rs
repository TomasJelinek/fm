use crate::glyph_cache::{GlyphCache, GlyphKey};
#[path = "outline_cache.rs"]
mod outlines;
#[path = "glyph_timing.rs"]
mod timing;
use crate::{
    LayoutError, RasterMut, TextPage, Viewport,
    raster::{blend_pixel, premultiply, stable_pixel},
};
use parley::{GlyphRun, Layout, PositionedLayoutItem};
use std::collections::HashMap;
use swash::{
    CacheKey, FontRef,
    scale::{
        Render, ScaleContext, Source, StrikeWith,
        image::{Content, Image},
    },
    zeno::{Transform, Vector},
};

/// Conservative ink reach of one glyph around its origin, in multiples of its
/// font size: wide enough for the bearings, accents, stacked marks (whose own
/// origins are tracked separately) and synthesis of real text fonts, but not a
/// font guarantee. Glyphs and lines beyond it are not painted; a malformed or
/// unusual font whose glyph ink reaches further can lose that ink at raster
/// edges. Decorations are culled by their own extents (see `line_bounds`).
const INK_EM_VERTICAL: f32 = 4.0;
const INK_EM_HORIZONTAL: f32 = 8.0;
const MAX_FONTS: usize = 256;
const MAX_VARIATIONS: usize = 1024;

#[derive(Default)]
pub(crate) struct GlyphPainter {
    scale: ScaleContext,
    /// Scaler cache identity and whether the font has color glyphs.
    fonts: HashMap<(u64, u32), (CacheKey, bool)>,
    /// Variation coordinates interned once per run; 0 is the default instance.
    variations: HashMap<Box<[i16]>, u32>,
    cache: GlyphCache,
    outlines: outlines::OutlineCache,
    #[cfg(test)]
    scales: usize,
    #[cfg(test)]
    uncached_outlines: bool,
    #[cfg(test)]
    unculled: bool,
    #[cfg(test)]
    misses: usize,
}

/// Vertical reach of each line: glyph origins (including stacked-mark offsets)
/// and baselines widened by the ink margin, plus the exact extent of every
/// strikethrough and underline, whose font- or style-supplied offsets need not
/// stay near the glyphs. Computed once per layout.
pub(crate) fn line_bounds(layout: &Layout<[u8; 4]>) -> Vec<[f32; 2]> {
    layout
        .lines()
        .map(|line| {
            let metrics = line.metrics();
            let mut bounds = [metrics.block_min_coord, metrics.block_max_coord];
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else {
                    continue;
                };
                let reach = run.run().font_size() * INK_EM_VERTICAL;
                let baseline = run.baseline();
                bounds = [
                    bounds[0].min(baseline - reach),
                    bounds[1].max(baseline + reach),
                ];
                for glyph in run.positioned_glyphs() {
                    bounds = [
                        bounds[0].min(glyph.y - reach),
                        bounds[1].max(glyph.y + reach),
                    ];
                }
                for (y, height, _) in decorations(&run) {
                    bounds = [bounds[0].min(y), bounds[1].max(y + height)];
                }
            }
            bounds
        })
        .collect()
}

impl TextPage {
    /// Vertical layout range that this page's glyph ink and decorations can
    /// reach: its own box widened by the reach of every line (see `line_bounds`).
    pub(crate) fn vertical_reach(&self) -> [f32; 2] {
        let union = |a: [f32; 2], b: [f32; 2]| [a[0].min(b[0]), a[1].max(b[1])];
        let own = [0.0, self.height()];
        match &self.composite {
            Some(grid) => grid.parts.iter().fold(own, |bounds, part| {
                let reach = part.page.vertical_reach();
                union(bounds, reach.map(|edge| part.origin[1] + edge))
            }),
            None => self.line_bounds.iter().copied().fold(own, union),
        }
    }
}

/// Strikethrough, then underline of a run: top edge, thickness (at least one
/// pixel) and brush, in layout coordinates.
fn decorations(run: &GlyphRun<'_, [u8; 4]>) -> impl Iterator<Item = (f32, f32, [u8; 4])> {
    let metrics = run.run().metrics();
    let style = run.style();
    let baseline = run.baseline();
    [
        (
            &style.strikethrough,
            metrics.strikethrough_offset,
            metrics.strikethrough_size,
        ),
        (
            &style.underline,
            metrics.underline_offset,
            metrics.underline_size,
        ),
    ]
    .into_iter()
    .filter_map(move |(decoration, offset, size)| {
        let decoration = decoration.as_ref()?;
        Some((
            baseline - decoration.offset.unwrap_or(offset),
            decoration.size.unwrap_or(size).max(1.0),
            decoration.brush,
        ))
    })
}

impl GlyphPainter {
    fn font_entry(
        &mut self,
        id: (u64, u32),
        data: &parley::FontData,
    ) -> Result<(CacheKey, bool), LayoutError> {
        if let Some(entry) = self.fonts.get(&id) {
            return Ok(*entry);
        }
        let mut font = FontRef::from_index(data.data.as_ref(), data.index as usize)
            .ok_or(LayoutError::InvalidFont)?;
        if self.fonts.len() >= MAX_FONTS {
            self.fonts.clear();
        }
        font.key = CacheKey::new();
        let scaler = self.scale.builder(font).build();
        // Color glyph images depend on the brush; coverage masks do not.
        let entry = (
            font.key,
            scaler.has_color_outlines() || scaler.has_color_bitmaps(),
        );
        self.fonts.insert(id, entry);
        Ok(entry)
    }

    fn variation(&mut self, coordinates: &[i16]) -> u32 {
        if coordinates.is_empty() {
            return 0;
        }
        if let Some(id) = self.variations.get(coordinates) {
            return *id;
        }
        if self.variations.len() >= MAX_VARIATIONS {
            // Ids identify cached images; never reassign one that a cache holds.
            self.variations.clear();
            self.cache.clear();
            self.outlines.clear();
        }
        let id = self.variations.len() as u32 + 1;
        self.variations.insert(coordinates.into(), id);
        id
    }
}

impl GlyphPainter {
    pub(crate) fn paint(
        &mut self,
        page: &TextPage,
        view: Viewport,
        raster: &mut RasterMut<'_>,
    ) -> Result<(), LayoutError> {
        self.paint_traced(page, view, raster, &mut timing::Trace::new())
    }

    fn paint_traced(
        &mut self,
        page: &TextPage,
        view: Viewport,
        raster: &mut RasterMut<'_>,
        trace: &mut timing::Trace,
    ) -> Result<(), LayoutError> {
        #[cfg(test)]
        let (outlines_enabled, culled) = (!self.uncached_outlines, !self.unculled);
        #[cfg(not(test))]
        let (outlines_enabled, culled) = (true, true);
        // Layout coordinates covered by this raster. Large pages are painted
        // once per guard refill; only parts, lines and glyphs that can reach it run.
        let top = view.origin[1];
        let bottom = top + view.height as f32;
        if let Some(grid) = &page.composite {
            for rect in &grid.borders {
                raster.fill(*rect, view.origin, grid.border_color);
            }
            for part in &grid.parts {
                // Marks stacked above a cell's first line reach beyond its box.
                let reach = part.page.vertical_reach();
                if culled && (part.origin[1] + reach[1] < top || part.origin[1] + reach[0] > bottom)
                {
                    continue;
                }
                self.paint_traced(&part.page, view.translated(part.origin), raster, trace)?;
            }
            return Ok(());
        }
        let left = view.origin[0];
        let right = left + view.width as f32;
        for (index, line) in page.layout.lines().enumerate() {
            if culled
                && page
                    .line_bounds
                    .get(index)
                    .is_some_and(|bounds| bounds[1] < top || bounds[0] > bottom)
            {
                continue;
            }
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                    continue;
                };
                let run = glyph_run.run();
                let reach = [
                    run.font_size() * INK_EM_HORIZONTAL,
                    run.font_size() * INK_EM_VERTICAL,
                ];
                if culled
                    && (glyph_run.offset() + glyph_run.advance() + reach[0] < left
                        || glyph_run.offset() - reach[0] > right)
                {
                    continue;
                }
                for (y, height, brush) in decorations(&glyph_run) {
                    raster.fill(
                        crate::Rect {
                            x0: f64::from(glyph_run.offset()),
                            y0: f64::from(y),
                            x1: f64::from(glyph_run.offset() + glyph_run.advance()),
                            y1: f64::from(y + height),
                        },
                        view.origin,
                        brush,
                    );
                }
                if run.normalized_coords().len() > 64 {
                    return Err(LayoutError::InvalidFont);
                }
                let data = run.font();
                let font_id = (data.data.id(), data.index);
                let (font_key, colored) = self.font_entry(font_id, data)?;
                let coordinates = self.variation(run.normalized_coords());
                let brush = glyph_run.style().brush;
                // Most scrolling frames reuse every glyph. Scaler setup belongs
                // to the cache-miss path, not to every styled run.
                let mut scaler = None;
                let synthesis = run.synthesis();
                for glyph in glyph_run.positioned_glyphs() {
                    if culled
                        && (glyph.x + reach[0] < left
                            || glyph.x - reach[0] > right
                            || glyph.y + reach[1] < top
                            || glyph.y - reach[1] > bottom)
                    {
                        continue;
                    }
                    let id = u16::try_from(glyph.id).map_err(|_| LayoutError::InvalidFont)?;
                    // Raster phase belongs to the final screen position. A
                    // source-page rebase must not change an overlapping glyph.
                    let x = stable_pixel(f64::from(glyph.x) - f64::from(view.origin[0]));
                    let y = stable_pixel(f64::from(glyph.y) - f64::from(view.origin[1]));
                    let fraction = [(x - x.floor()) as f32, (y - y.floor()) as f32];
                    let key = GlyphKey {
                        font: font_id,
                        size: run.font_size().to_bits(),
                        coordinates,
                        glyph: id,
                        fraction: fraction.map(f32::to_bits),
                        color: if colored { brush } else { [0; 4] },
                        embolden: synthesis.embolden(),
                        skew: synthesis.skew().unwrap_or(0.0).to_bits(),
                    };
                    let x = x.floor();
                    let y = y.floor();
                    if let Some(image) = self.cache.get(&key) {
                        if let Some(image) = image {
                            trace.compose(|| paint_image(raster, image, x, y, brush));
                        }
                        continue;
                    }
                    #[cfg(test)]
                    {
                        self.misses += 1;
                    }
                    if outlines_enabled
                        && let Some(outline) = trace.generate(|| {
                            self.outlines
                                .get(&key, [x, y], [raster.width, raster.height])
                        })
                    {
                        let at = (key, [x, y]);
                        present_outline(&mut self.cache, raster, trace, at, outline, brush);
                        continue;
                    }
                    if scaler.is_none() {
                        #[cfg(test)]
                        {
                            self.scales += 1;
                        }
                        let mut font = FontRef::from_index(data.data.as_ref(), data.index as usize)
                            .ok_or(LayoutError::InvalidFont)?;
                        font.key = font_key;
                        scaler = Some(
                            self.scale
                                .builder(font)
                                .size(run.font_size())
                                .hint(true)
                                .normalized_coords(run.normalized_coords())
                                .build(),
                        );
                    }
                    if outlines_enabled
                        && let Some(outline) = trace.generate(|| {
                            Ok::<_, LayoutError>(self.outlines.build(
                                &key,
                                scaler.as_mut().ok_or(LayoutError::InvalidFont)?,
                                [x, y],
                                [raster.width, raster.height],
                            ))
                        })?
                    {
                        let at = (key, [x, y]);
                        present_outline(&mut self.cache, raster, trace, at, outline, brush);
                        continue;
                    }
                    let mut render = Render::new(&[
                        Source::ColorOutline(0),
                        Source::ColorBitmap(StrikeWith::BestFit),
                        Source::Outline,
                        Source::Bitmap(StrikeWith::BestFit),
                    ]);
                    render.offset(Vector::new(fraction[0], -fraction[1]));
                    render.default_color(brush);
                    if synthesis.embolden() {
                        render.embolden(run.font_size() * 0.02);
                    }
                    if let Some(degrees) = synthesis.skew() {
                        render.transform(Some(Transform::new(
                            1.0,
                            0.0,
                            degrees.to_radians().tan(),
                            1.0,
                            0.0,
                            0.0,
                        )));
                    }
                    let font_scaler = scaler.as_mut().ok_or(LayoutError::InvalidFont)?;
                    let image = trace.generate(|| render.render(font_scaler, id));
                    if let Some(image) = &image {
                        trace.compose(|| paint_image(raster, image, x, y, brush));
                    }
                    self.cache.insert(key, image);
                }
            }
        }
        Ok(())
    }
}

/// Paint a cached outline's phase image at its floored raster pixel and cache
/// what is valid for that phase.
fn present_outline(
    cache: &mut GlyphCache,
    raster: &mut RasterMut<'_>,
    trace: &mut timing::Trace,
    (key, [x, y]): (GlyphKey, [f64; 2]),
    outline: outlines::Outline,
    brush: [u8; 4],
) {
    match outline {
        outlines::Outline::Ink(image) => {
            trace.compose(|| paint_image(raster, &image, x, y, brush));
            cache.insert(key, Some(image));
        }
        // A path without ink is invisible at every phase, like an empty image.
        outlines::Outline::Empty => cache.insert(key, None),
        // The same glyph/phase can become visible after scrolling; do not
        // poison the phase image cache with this raster's clipping.
        outlines::Outline::Offscreen => {}
    }
}

fn paint_image(raster: &mut RasterMut<'_>, image: &Image, x: f64, y: f64, brush: [u8; 4]) {
    let left = x + f64::from(image.placement.left);
    let top = y - f64::from(image.placement.top);
    let width = f64::from(image.placement.width);
    let height = f64::from(image.placement.height);
    if left >= f64::from(raster.width)
        || left + width <= 0.0
        || top >= f64::from(raster.height)
        || top + height <= 0.0
    {
        return;
    }
    // Intersection proves these conversions are bounded by the image dimensions.
    let left = left.floor() as i64;
    let top = top.floor() as i64;
    let x0 = (-left).max(0) as usize;
    let y0 = (-top).max(0) as usize;
    let x1 = (i64::from(raster.width) - left).min(i64::from(image.placement.width)) as usize;
    let y1 = (i64::from(raster.height) - top).min(i64::from(image.placement.height)) as usize;
    if image.content == Content::Mask {
        paint_mask(raster, image, [left, top], [x0, y0], [x1, y1], brush);
        return;
    }
    for y in y0..y1 {
        for x in x0..x1 {
            let source = (y * image.placement.width as usize + x) * 4;
            let color = match image.content {
                Content::Color => {
                    let color = [
                        image.data[source],
                        image.data[source + 1],
                        image.data[source + 2],
                        image.data[source + 3],
                    ];
                    if matches!(image.source, Source::ColorOutline(_)) {
                        color
                    } else {
                        premultiply(color)
                    }
                }
                // Alpha rendering above never requests LCD subpixel masks.
                Content::SubpixelMask | Content::Mask => continue,
            };
            raster.blend((left + x as i64) as usize, (top + y as i64) as usize, color);
        }
    }
}

// Bounds are the same clipped image intersection used by the color path above.
// Each row pairs contiguous coverage and RGBA pixels, avoiding per-pixel indexing.
fn paint_mask(
    raster: &mut RasterMut<'_>,
    image: &Image,
    origin: [i64; 2],
    start: [usize; 2],
    end: [usize; 2],
    brush: [u8; 4],
) {
    if brush[3] == 0 {
        return;
    }
    let count = end[0] - start[0];
    let left = (origin[0] + start[0] as i64) as usize;
    for y in start[1]..end[1] {
        let source = y * image.placement.width as usize + start[0];
        let target = ((origin[1] + y as i64) as usize * raster.width as usize + left) * 4;
        let masks = &image.data[source..source + count];
        let pixels = raster.rgba[target..target + count * 4]
            .as_chunks_mut::<4>()
            .0;
        for (&coverage, pixel) in masks.iter().zip(pixels) {
            let alpha = ((u16::from(coverage) * u16::from(brush[3]) + 127) / 255) as u8;
            match alpha {
                0 => {}
                255 => *pixel = brush,
                _ => blend_pixel(pixel, premultiply([brush[0], brush[1], brush[2], alpha])),
            }
        }
    }
}

#[cfg(test)]
#[path = "outline_tests.rs"]
mod outline_tests;

#[cfg(test)]
#[path = "glyph_pixel_tests.rs"]
mod pixel_tests;
