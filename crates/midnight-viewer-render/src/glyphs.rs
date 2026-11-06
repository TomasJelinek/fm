use crate::glyph_cache::{GlyphCache, GlyphKey};
use crate::{LayoutError, Raster, TextPage, Viewport, raster::premultiply};
use parley::PositionedLayoutItem;
use std::collections::HashMap;
use swash::{
    CacheKey, FontRef,
    scale::{
        Render, ScaleContext, Source, StrikeWith,
        image::{Content, Image},
    },
    zeno::{Transform, Vector},
};

#[derive(Default)]
pub(crate) struct GlyphPainter {
    scale: ScaleContext,
    fonts: HashMap<(u64, u32), CacheKey>,
    cache: GlyphCache,
}

impl GlyphPainter {
    pub(crate) fn paint(
        &mut self,
        page: &TextPage,
        view: Viewport,
        raster: &mut Raster,
    ) -> Result<(), LayoutError> {
        for line in page.layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                    continue;
                };
                let run = glyph_run.run();
                if run.normalized_coords().len() > 64 {
                    return Err(LayoutError::InvalidFont);
                }
                let data = run.font();
                let mut font = FontRef::from_index(data.data.as_ref(), data.index as usize)
                    .ok_or(LayoutError::InvalidFont)?;
                if self.fonts.len() >= 256 {
                    self.fonts.clear();
                }
                let font_id = (data.data.id(), data.index);
                font.key = *self.fonts.entry(font_id).or_default();
                let mut scaler = self
                    .scale
                    .builder(font)
                    .size(run.font_size())
                    .hint(true)
                    .normalized_coords(run.normalized_coords())
                    .build();
                let synthesis = run.synthesis();
                for glyph in glyph_run.positioned_glyphs() {
                    let id = u16::try_from(glyph.id).map_err(|_| LayoutError::InvalidFont)?;
                    let key = GlyphKey {
                        font: font_id,
                        size: run.font_size().to_bits(),
                        coordinates: run.normalized_coords().to_vec(),
                        glyph: id,
                        fraction: [glyph.x.fract().to_bits(), glyph.y.fract().to_bits()],
                        color: glyph_run.style().brush,
                        embolden: synthesis.embolden(),
                        skew: synthesis.skew().unwrap_or(0.0).to_bits(),
                    };
                    let x = f64::from(glyph.x.floor()) - f64::from(view.origin[0]);
                    let y = f64::from(glyph.y.floor()) - f64::from(view.origin[1]);
                    if let Some(image) = self.cache.get(&key) {
                        if let Some(image) = image {
                            paint_image(raster, image, x, y, key.color);
                        }
                        continue;
                    }
                    let mut render = Render::new(&[
                        Source::ColorOutline(0),
                        Source::ColorBitmap(StrikeWith::BestFit),
                        Source::Outline,
                        Source::Bitmap(StrikeWith::BestFit),
                    ]);
                    render.offset(Vector::new(glyph.x.fract(), -glyph.y.fract()));
                    render.default_color(glyph_run.style().brush);
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
                    let image = render.render(&mut scaler, id);
                    if let Some(image) = &image {
                        paint_image(raster, image, x, y, key.color);
                    }
                    self.cache.insert(key, image);
                }
            }
        }
        Ok(())
    }
}

fn paint_image(raster: &mut Raster, image: &Image, x: f64, y: f64, brush: [u8; 4]) {
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
    let channels = if image.content == Content::Mask { 1 } else { 4 };
    for y in y0..y1 {
        for x in x0..x1 {
            let source = (y * image.placement.width as usize + x) * channels;
            let color = match image.content {
                Content::Mask => premultiply([
                    brush[0],
                    brush[1],
                    brush[2],
                    ((u16::from(image.data[source]) * u16::from(brush[3]) + 127) / 255) as u8,
                ]),
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
                Content::SubpixelMask => continue,
            };
            raster.blend((left + x as i64) as usize, (top + y as i64) as usize, color);
        }
    }
}
