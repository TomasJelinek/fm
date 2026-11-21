use crate::glyph_cache::{GlyphCache, GlyphKey};
#[path = "outline_cache.rs"]
mod outlines;
#[path = "glyph_timing.rs"]
mod timing;
use crate::{
    LayoutError, RasterMut, TextPage, Viewport,
    contrast::SelectionColors,
    raster::{blend_pixel, premultiply, stable_pixel},
    tables::intersection,
};
use parley::{GlyphRun, Layout, PositionedLayoutItem};
use std::{collections::HashMap, ops::Range};
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
    /// Readable brushes of selected glyphs over the current selection color.
    selection_colors: SelectionColors,
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

/// Per glyph of a run in visual order, after its first `skip` glyphs: whether the glyph's
/// cluster starts inside `selection` (layout bytes). Ligature and mark glyphs follow their
/// cluster.
fn selected_glyphs<'a>(
    run: &'a parley::Run<'a, [u8; 4]>,
    selection: Range<usize>,
    skip: usize,
) -> impl Iterator<Item = bool> + 'a {
    run.visual_clusters()
        .flat_map(move |cluster| {
            let inside = selection.contains(&cluster.text_range().start);
            cluster.glyphs().map(move |_| inside)
        })
        .skip(skip)
}

/// The stretches along `glyph_run`'s baseline whose glyphs are selected or not, where its
/// underline and strikethrough take the selected or the run's brush: the whole run unless
/// it is partly selected. `first_glyph` is as in `selected_glyphs`.
fn decoration_spans(
    glyph_run: &GlyphRun<'_, [u8; 4]>,
    coverage: Coverage,
    selection: Option<&Range<usize>>,
    first_glyph: usize,
) -> Vec<(f32, f32, bool)> {
    let start = glyph_run.offset();
    let (Coverage::Partial, Some(selection)) = (coverage, selection) else {
        let end = start + glyph_run.advance();
        return vec![(start, end, coverage == Coverage::Selected)];
    };
    let mut spans: Vec<(f32, f32, bool)> = Vec::new();
    let mut position = start;
    let flags = selected_glyphs(glyph_run.run(), selection.clone(), first_glyph);
    for (glyph, inside) in glyph_run.glyphs().zip(flags) {
        let end = position + glyph.advance;
        match spans.last_mut() {
            Some(span) if span.2 == inside => span.1 = end,
            _ => spans.push((position, end, inside)),
        }
        position = end;
    }
    spans
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

/// How much of one run's text a selection covers.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Coverage {
    Unselected,
    Selected,
    Partial,
}

impl Coverage {
    /// `selection` and `run` are both layout byte ranges.
    fn of(selection: Option<&Range<usize>>, run: Range<usize>) -> Self {
        match selection {
            Some(selection) if selection.start < run.end && run.start < selection.end => {
                if selection.start <= run.start && run.end <= selection.end {
                    Self::Selected
                } else {
                    Self::Partial
                }
            }
            _ => Self::Unselected,
        }
    }
}

impl GlyphPainter {
    /// Paint the glyphs of `page`. A glyph whose cluster starts inside
    /// `selection` (decoded UTF-8 bytes of `page`) uses `view.selection_text`, or its
    /// own brush when that is transparent, readable over `view.selection_color`
    /// painted on `view.background`.
    pub(crate) fn paint(
        &mut self,
        page: &TextPage,
        view: Viewport,
        selection: Option<Range<usize>>,
        raster: &mut RasterMut<'_>,
    ) -> Result<(), LayoutError> {
        // Only a visible selection changes glyph colors.
        let selection = match selection {
            Some(range)
                if !range.is_empty()
                    && self.selection_colors.prepare(
                        view.selection_color,
                        view.background,
                        view.selection_text,
                    ) =>
            {
                Some(range)
            }
            _ => None,
        };
        self.paint_traced(
            page,
            view,
            None,
            selection,
            raster,
            &mut timing::Trace::new(),
        )
    }

    pub(crate) fn paint_annotation(
        &mut self,
        page: &TextPage,
        view: Viewport,
        origin: [f64; 2],
        raster: &mut RasterMut<'_>,
    ) -> Result<(), LayoutError> {
        let trace = &mut timing::Trace::new();
        self.paint_traced(page, view, Some(origin), None, raster, trace)
    }

    fn paint_traced(
        &mut self,
        page: &TextPage,
        view: Viewport,
        precise_origin: Option<[f64; 2]>,
        selection: Option<Range<usize>>,
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
        let origin = precise_origin.unwrap_or_else(|| view.origin.map(f64::from));
        let precise_bounds = precise_origin.map(|origin| {
            [
                origin[0],
                origin[1],
                origin[0] + f64::from(view.width),
                origin[1] + f64::from(view.height),
            ]
        });
        if let Some(grid) = &page.composite {
            for rect in &grid.borders {
                raster.fill_at_origin(*rect, origin, grid.border_color);
            }
            for part in &grid.parts {
                // Marks stacked above a cell's first line reach beyond its box.
                let reach = part.page.vertical_reach();
                if culled
                    && match precise_bounds {
                        Some(bounds) => {
                            f64::from(part.origin[1]) + f64::from(reach[1]) < bounds[1]
                                || f64::from(part.origin[1]) + f64::from(reach[0]) > bounds[3]
                        }
                        None => {
                            part.origin[1] + reach[1] < top || part.origin[1] + reach[0] > bottom
                        }
                    }
                {
                    continue;
                }
                let translated = precise_origin.map(|origin| {
                    std::array::from_fn(|axis| origin[axis] - f64::from(part.origin[axis]))
                });
                let local = selection
                    .as_ref()
                    .and_then(|range| intersection(range, &part.range));
                self.paint_traced(
                    &part.page,
                    view.translated(part.origin),
                    translated,
                    local,
                    raster,
                    trace,
                )?;
            }
            return Ok(());
        }
        // Shaping expands tabs and CRLF; clusters carry these layout bytes.
        let selection = selection.map(|range| {
            page.source_map.to_layout(range.start)..page.source_map.to_layout(range.end)
        });
        let left = view.origin[0];
        let right = left + view.width as f32;
        for (index, line) in page.layout.lines().enumerate() {
            if culled
                && page
                    .line_bounds
                    .get(index)
                    .is_some_and(|bounds| match precise_bounds {
                        Some(clip) => {
                            f64::from(bounds[1]) < clip[1] || f64::from(bounds[0]) > clip[3]
                        }
                        None => bounds[1] < top || bounds[0] > bottom,
                    })
            {
                continue;
            }
            // Parley splits a shaped run into consecutive glyph runs by style:
            // the run (index in the line) and its glyphs before this glyph run.
            let mut preceding = (usize::MAX, 0);
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                    continue;
                };
                let run = glyph_run.run();
                let coverage = Coverage::of(selection.as_ref(), run.text_range());
                let first_glyph = if coverage == Coverage::Partial {
                    if preceding.0 != run.index() {
                        preceding = (run.index(), 0);
                    }
                    let first = preceding.1;
                    preceding.1 += glyph_run.glyphs().count();
                    first
                } else {
                    0
                };
                let reach = [
                    run.font_size() * INK_EM_HORIZONTAL,
                    run.font_size() * INK_EM_VERTICAL,
                ];
                if culled
                    && match precise_bounds {
                        Some(bounds) => {
                            f64::from(glyph_run.offset())
                                + f64::from(glyph_run.advance())
                                + f64::from(reach[0])
                                < bounds[0]
                                || f64::from(glyph_run.offset()) - f64::from(reach[0]) > bounds[2]
                        }
                        None => {
                            glyph_run.offset() + glyph_run.advance() + reach[0] < left
                                || glyph_run.offset() - reach[0] > right
                        }
                    }
                {
                    continue;
                }
                let mut lines = decorations(&glyph_run).peekable();
                if lines.peek().is_some() {
                    // Underlines and strikethroughs of selected text stay readable too.
                    let spans =
                        decoration_spans(&glyph_run, coverage, selection.as_ref(), first_glyph);
                    for (y, height, brush) in lines {
                        let selected_line = match coverage {
                            Coverage::Unselected => brush,
                            _ => self.selection_colors.brush(brush),
                        };
                        for &(x0, x1, inside) in &spans {
                            raster.fill_at_origin(
                                crate::Rect {
                                    x0: f64::from(x0),
                                    y0: f64::from(y),
                                    x1: f64::from(x1),
                                    y1: f64::from(y + height),
                                },
                                origin,
                                if inside { selected_line } else { brush },
                            );
                        }
                    }
                }
                if run.normalized_coords().len() > 64 {
                    return Err(LayoutError::InvalidFont);
                }
                let data = run.font();
                let font_id = (data.data.id(), data.index);
                let (font_key, colored) = self.font_entry(font_id, data)?;
                let coordinates = self.variation(run.normalized_coords());
                let run_brush = glyph_run.style().brush;
                let selected_brush = match coverage {
                    Coverage::Unselected => run_brush,
                    _ => self.selection_colors.brush(run_brush),
                };
                // A partly selected run decides per glyph: is its cluster's first
                // byte selected? Ligature and mark glyphs follow their cluster.
                let mut clusters = (coverage == Coverage::Partial).then(|| {
                    selected_glyphs(run, selection.clone().unwrap_or_default(), first_glyph)
                });
                // Most scrolling frames reuse every glyph. Scaler setup belongs
                // to the cache-miss path, not to every styled run.
                let mut scaler = None;
                let synthesis = run.synthesis();
                for glyph in glyph_run.positioned_glyphs() {
                    // Consume each glyph's cluster flag before culling can skip it.
                    let inside = match &mut clusters {
                        Some(clusters) => clusters.next().unwrap_or(false),
                        None => coverage == Coverage::Selected,
                    };
                    let brush = if inside { selected_brush } else { run_brush };
                    if culled
                        && match precise_bounds {
                            Some(bounds) => {
                                f64::from(glyph.x) + f64::from(reach[0]) < bounds[0]
                                    || f64::from(glyph.x) - f64::from(reach[0]) > bounds[2]
                                    || f64::from(glyph.y) + f64::from(reach[1]) < bounds[1]
                                    || f64::from(glyph.y) - f64::from(reach[1]) > bounds[3]
                            }
                            None => {
                                glyph.x + reach[0] < left
                                    || glyph.x - reach[0] > right
                                    || glyph.y + reach[1] < top
                                    || glyph.y - reach[1] > bottom
                            }
                        }
                    {
                        continue;
                    }
                    let id = u16::try_from(glyph.id).map_err(|_| LayoutError::InvalidFont)?;
                    // Raster phase belongs to the final screen position. A
                    // source-page rebase must not change an overlapping glyph.
                    let x = stable_pixel(f64::from(glyph.x) - origin[0]);
                    let y = stable_pixel(f64::from(glyph.y) - origin[1]);
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
