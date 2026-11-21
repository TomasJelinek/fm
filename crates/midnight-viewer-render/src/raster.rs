use crate::{LayoutError, Rect, TextPage, TextRenderer};
use std::ops::Range;

#[derive(Clone, Copy)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
    pub origin: [f32; 2],
    pub background: [u8; 4],
    pub selection_color: [u8; 4],
    /// The color of selected glyphs and their underlines over `selection_color`.
    /// Fully transparent keeps each glyph's own color. Either way a color below
    /// 4.5:1 on the painted selection moves in lightness until it reaches that.
    pub selection_text: [u8; 4],
}

impl Viewport {
    pub(crate) fn translated(self, offset: [f32; 2]) -> Self {
        Self {
            origin: std::array::from_fn(|axis| {
                stable_pixel(f64::from(self.origin[axis] - offset[axis])) as f32
            }),
            ..self
        }
    }
}

// A caret restored through different composite parents can leave sub-millipixel
// cancellation error. Normalize final paint coordinates, including root borders,
// without discarding meaningful fractional scrolling.
pub(crate) fn stable_pixel(value: f64) -> f64 {
    let pixel = value.round();
    if (value - pixel).abs() < 1.0 / 1024.0 {
        pixel
    } else {
        value
    }
}

pub struct Raster {
    pub width: u32,
    pub height: u32,
    /// Premultiplied RGBA, ready for the shared host surface.
    pub rgba: Vec<u8>,
}

/// A borrowed premultiplied RGBA surface of exactly `width * height` pixels,
/// for example the still unique buffer of a frame shared later as `Arc<[u8]>`.
/// Painting into it avoids a second allocation and copy of the whole surface.
pub struct RasterMut<'a> {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) rgba: &'a mut [u8],
}

impl<'a> RasterMut<'a> {
    /// Borrow `rgba` as a `width` x `height` surface. Fails with
    /// [`LayoutError::InvalidGeometry`] unless it holds exactly that many pixels.
    pub fn new(width: u32, height: u32, rgba: &'a mut [u8]) -> Result<Self, LayoutError> {
        if pixel_bytes(width, height) != Some(rgba.len()) {
            return Err(LayoutError::InvalidGeometry);
        }
        Ok(Self {
            width,
            height,
            rgba,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Premultiplied RGBA rows, `width * 4` bytes each.
    pub fn rgba_mut(&mut self) -> &mut [u8] {
        self.rgba
    }
}

pub(crate) struct Fill {
    pub rect: Rect,
    pub color: [u8; 4],
    pub radius: f32,
}

const MAX_PIXELS: usize = 16 * 1024 * 1024;

/// A selected byte range and the layout rectangles that show it.
type Selection = (Range<usize>, Vec<Rect>);

fn pixel_bytes(width: u32, height: u32) -> Option<usize> {
    (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)
}

impl TextRenderer {
    /// Paint a prepared annotation over an existing surface, without clearing it,
    /// reshaping or allocating another viewport: its glyphs are blended over the
    /// current pixels. Annotation coordinates are physical pixels. A non-finite
    /// `origin` fails with [`LayoutError::InvalidGeometry`] before anything is painted.
    pub fn paint_over_into(
        &mut self,
        page: &TextPage,
        target: &mut RasterMut<'_>,
        origin: [f32; 2],
    ) -> Result<(), LayoutError> {
        if origin.iter().any(|value| !value.is_finite()) {
            return Err(LayoutError::InvalidGeometry);
        }
        self.glyphs.paint(
            page,
            Viewport {
                width: target.width,
                height: target.height,
                origin,
                background: [0; 4],
                selection_color: [0; 4],
                selection_text: [0; 4],
            },
            None,
            target,
        )
    }

    /// Blend an annotation at `annotation_position` relative to `camera_origin`.
    /// Both vectors use physical pixels. Their subtraction stays in f64 through
    /// clipping and glyph phase selection, so integer guard translations preserve
    /// the same pixels. This neither clears the target nor reshapes the page or
    /// paints its backgrounds. Invalid geometry fails before painting any pixels.
    pub fn paint_annotation_at_into(
        &mut self,
        page: &TextPage,
        target: &mut RasterMut<'_>,
        camera_origin: [f32; 2],
        annotation_position: [f32; 2],
    ) -> Result<(), LayoutError> {
        if camera_origin
            .iter()
            .chain(&annotation_position)
            .any(|value| !value.is_finite())
        {
            return Err(LayoutError::InvalidGeometry);
        }
        let origin = std::array::from_fn(|axis| {
            f64::from(camera_origin[axis]) - f64::from(annotation_position[axis])
        });
        let viewport = Viewport {
            width: target.width,
            height: target.height,
            origin: origin.map(|value| value as f32),
            background: [0; 4],
            selection_color: [0; 4],
            selection_text: [0; 4],
        };
        let (bytes, _) = validate(page, viewport, None)?;
        if target.rgba.len() != bytes {
            return Err(LayoutError::InvalidGeometry);
        }
        self.glyphs.paint_annotation(page, viewport, origin, target)
    }

    /// Paint `page` into a new viewport raster. A `selection` (decoded UTF-8 byte
    /// range) is filled with `viewport.selection_color`; its glyphs take
    /// `viewport.selection_text` (see [`Viewport`]) and stay readable on that fill.
    pub fn rasterize(
        &mut self,
        page: &TextPage,
        viewport: Viewport,
        selection: Option<Range<usize>>,
    ) -> Result<Raster, LayoutError> {
        let (bytes, selection) = validate(page, viewport, selection)?;
        let mut rgba = vec![0; bytes];
        self.paint_viewport(
            page,
            viewport,
            selection,
            &mut RasterMut {
                width: viewport.width,
                height: viewport.height,
                rgba: &mut rgba,
            },
        )?;
        Ok(Raster {
            width: viewport.width,
            height: viewport.height,
            rgba,
        })
    }

    /// [`Self::rasterize`] into a caller-owned buffer of exactly
    /// `viewport.width * viewport.height * 4` bytes, which is overwritten
    /// completely with the same bytes. A buffer of another length fails with
    /// [`LayoutError::InvalidGeometry`] before anything is written; after the
    /// errors that `rasterize` can report, the buffer contents are unspecified.
    pub fn rasterize_into(
        &mut self,
        page: &TextPage,
        viewport: Viewport,
        selection: Option<Range<usize>>,
        rgba: &mut [u8],
    ) -> Result<(), LayoutError> {
        let (bytes, selection) = validate(page, viewport, selection)?;
        if rgba.len() != bytes {
            return Err(LayoutError::InvalidGeometry);
        }
        self.paint_viewport(
            page,
            viewport,
            selection,
            &mut RasterMut {
                width: viewport.width,
                height: viewport.height,
                rgba,
            },
        )
    }

    fn paint_viewport(
        &mut self,
        page: &TextPage,
        viewport: Viewport,
        selection: Option<Selection>,
        raster: &mut RasterMut<'_>,
    ) -> Result<(), LayoutError> {
        let mut trace = cfg!(debug_assertions)
            .then(|| std::env::var_os("MIDNIGHT_VIEWER_SCROLL_TRACE"))
            .flatten()
            .map(|_| std::time::Instant::now());
        let background = premultiply(viewport.background);
        raster.rgba.as_chunks_mut::<4>().0.fill(background);
        trace_phase("clear", &mut trace);
        paint_backgrounds(page, viewport, raster);
        let selection = selection.map(|(range, rectangles)| {
            for rect in rectangles {
                raster.fill(rect, viewport.origin, viewport.selection_color);
            }
            range
        });
        trace_phase("backgrounds", &mut trace);
        self.glyphs.paint(page, viewport, selection, raster)?;
        trace_phase("glyphs", &mut trace);
        Ok(())
    }
}

/// Byte length of a viewport raster and the selected range with its rectangles,
/// validated before any pixel is allocated or written.
fn validate(
    page: &TextPage,
    viewport: Viewport,
    selection: Option<Range<usize>>,
) -> Result<(usize, Option<Selection>), LayoutError> {
    let bytes = (viewport.width as usize)
        .checked_mul(viewport.height as usize)
        .filter(|&n| n > 0 && n <= MAX_PIXELS)
        .ok_or(LayoutError::LimitExceeded)?
        * 4;
    if viewport.origin.iter().any(|value| !value.is_finite()) {
        return Err(LayoutError::InvalidGeometry);
    }
    let selection = selection
        .map(|range| {
            let rectangles = page.selection_geometry(range.clone())?;
            Ok((range, rectangles))
        })
        .transpose()?;
    Ok((bytes, selection))
}

impl RasterMut<'_> {
    fn fill_rounded(&mut self, fill: &Fill, origin: [f32; 2]) {
        if fill.radius <= 0.0 {
            self.fill(fill.rect, origin, fill.color);
            return;
        }
        let x0 = stable_pixel(fill.rect.x0 - f64::from(origin[0]));
        let x1 = stable_pixel(fill.rect.x1 - f64::from(origin[0]));
        let y0 = stable_pixel(fill.rect.y0 - f64::from(origin[1]));
        let y1 = stable_pixel(fill.rect.y1 - f64::from(origin[1]));
        let radius = f64::from(fill.radius)
            .min((x1 - x0) / 2.0)
            .min((y1 - y0) / 2.0);
        let color = premultiply(fill.color);
        let left = x0.floor().clamp(0.0, f64::from(self.width)) as usize;
        let right = x1.ceil().clamp(0.0, f64::from(self.width)) as usize;
        if left >= right {
            return;
        }
        // Between the corner arcs dx is zero, so every pixel in this row has
        // exactly the same coverage. Keep scalar corner math and batch the body.
        let middle_left = (x0 + radius - 0.5).ceil().clamp(left as f64, right as f64) as usize;
        let middle_right =
            ((x1 - radius - 0.5).floor() + 1.0).clamp(middle_left as f64, right as f64) as usize;
        for y in
            y0.floor().max(0.0) as usize..y1.ceil().min(f64::from(self.height)).max(0.0) as usize
        {
            let py = y as f64 + 0.5;
            let dy = (y0 + radius - py).max(0.0).max(py - y1 + radius);
            let coverage = (radius + 0.5 - dy).clamp(0.0, 1.0);
            self.blend_span(
                y,
                middle_left,
                middle_right,
                color.map(|channel| (f64::from(channel) * coverage).round() as u8),
            );
            for x in (left..middle_left).chain(middle_right..right) {
                let px = x as f64 + 0.5;
                let dx = (x0 + radius - px).max(0.0).max(px - x1 + radius);
                let coverage = (radius + 0.5 - dx.hypot(dy)).clamp(0.0, 1.0);
                self.blend(
                    x,
                    y,
                    color.map(|channel| (f64::from(channel) * coverage).round() as u8),
                );
            }
        }
    }
    pub(crate) fn fill(&mut self, rect: Rect, origin: [f32; 2], color: [u8; 4]) {
        self.fill_at_origin(rect, origin.map(f64::from), color);
    }

    pub(crate) fn fill_at_origin(&mut self, rect: Rect, origin: [f64; 2], color: [u8; 4]) {
        let x0 = stable_pixel(rect.x0 - origin[0])
            .floor()
            .clamp(0.0, f64::from(self.width)) as usize;
        let x1 = stable_pixel(rect.x1 - origin[0])
            .ceil()
            .clamp(0.0, f64::from(self.width)) as usize;
        let y0 = stable_pixel(rect.y0 - origin[1])
            .floor()
            .clamp(0.0, f64::from(self.height)) as usize;
        let y1 = stable_pixel(rect.y1 - origin[1])
            .ceil()
            .clamp(0.0, f64::from(self.height)) as usize;
        let color = premultiply(color);
        for y in y0..y1 {
            self.blend_span(y, x0, x1, color);
        }
    }

    fn blend_span(&mut self, y: usize, left: usize, right: usize, color: [u8; 4]) {
        if left >= right {
            return;
        }
        if color[3] == 255 {
            let row = y * self.width as usize;
            self.rgba[(row + left) * 4..(row + right) * 4]
                .as_chunks_mut::<4>()
                .0
                .fill(color);
        } else if color[3] != 0 {
            for x in left..right {
                self.blend(x, y, color);
            }
        }
    }

    pub(crate) fn blend(&mut self, x: usize, y: usize, color: [u8; 4]) {
        let target = &mut self.rgba.as_chunks_mut::<4>().0[y * self.width as usize + x];
        blend_pixel(target, color);
    }
}

#[inline]
pub(crate) fn blend_pixel(target: &mut [u8; 4], color: [u8; 4]) {
    let remaining = 255 - u16::from(color[3]);
    for (target, source) in target.iter_mut().zip(color) {
        *target = (u16::from(source) + (u16::from(*target) * remaining + 127) / 255).min(255) as u8;
    }
}

fn paint_backgrounds(page: &TextPage, view: Viewport, raster: &mut RasterMut<'_>) {
    if let Some(grid) = &page.composite {
        for fill in &grid.backgrounds {
            raster.fill_rounded(fill, view.origin);
        }
        for part in &grid.parts {
            if part.origin[1] + part.page.height() >= view.origin[1]
                && part.origin[1] <= view.origin[1] + view.height as f32
            {
                paint_backgrounds(&part.page, view.translated(part.origin), raster);
            }
        }
    }
}

pub(crate) fn premultiply(color: [u8; 4]) -> [u8; 4] {
    let alpha = u16::from(color[3]);
    let channel = |value| ((u16::from(value) * alpha + 127) / 255) as u8;
    [
        channel(color[0]),
        channel(color[1]),
        channel(color[2]),
        color[3],
    ]
}

fn trace_phase(phase: &str, started: &mut Option<std::time::Instant>) {
    if let Some(started) = started {
        eprintln!(
            "raster_phase phase={phase} elapsed_ms={:.3}",
            started.elapsed().as_secs_f64() * 1000.0
        );
        *started = std::time::Instant::now();
    }
}

#[cfg(test)]
#[path = "raster_fill_tests.rs"]
mod fill_tests;

#[cfg(test)]
#[path = "annotation_tests.rs"]
mod annotation_tests;
